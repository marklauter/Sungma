//! Network Time Security, RFC 8915.
//!
//! A key exchange over TLS 1.3, on TCP port 4460, agrees two keys and hands
//! the client cookies. Each NTP request then carries one cookie, a unique
//! identifier and an AES-SIV authenticator. The server's reply is
//! authenticated with the other key, and carries fresh cookies inside its
//! encrypted part, so a forged or altered reply is refused.

use std::{
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream},
    sync::Arc,
    time::{Duration, Instant},
};

use aes_siv::{
    Aes128SivAead, KeyInit,
    aead::{Aead, Key, Nonce, Payload},
};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned, pki_types::ServerName};

use crate::{ClockError, Resolve, servers::resolve_by};

/// The key exchange's port.
pub(crate) const KE_PORT: u16 = 4460;
/// The longest a whole key exchange may take, so a server that trickles
/// bytes can't stall a sync.
pub(crate) const KE_DEADLINE: Duration = Duration::from_secs(5);

/// The most a key exchange response may hold, so a server can't stream
/// records without end.
const KE_LIMIT: usize = 64 * 1024;
const ALPN: &[u8] = b"ntske/1";
const EXPORTER: &[u8] = b"EXPORTER-network-time-security";

/// NTPv4, the only next protocol.
const NTPV4: u16 = 0;
/// AEAD_AES_SIV_CMAC_256, the only algorithm every server supports.
const AES_SIV: u16 = 15;

/// Key exchange record types.
const END: u16 = 0;
const NEXT_PROTOCOL: u16 = 1;
const ERROR: u16 = 2;
const WARNING: u16 = 3;
const AEAD: u16 = 4;
const NEW_COOKIE: u16 = 5;
const SERVER: u16 = 6;
const PORT: u16 = 7;
const CRITICAL: u16 = 0x8000;

/// NTP extension field types.
pub(crate) const UNIQUE_ID: u16 = 0x0104;
pub(crate) const COOKIE: u16 = 0x0204;
pub(crate) const PLACEHOLDER: u16 = 0x0304;
pub(crate) const AUTHENTICATOR: u16 = 0x0404;

/// How many cookies the client keeps, as RFC 8915 recommends.
const COOKIES: usize = 8;

/// The TLS settings for key exchange: TLS 1.3 only, ALPN `ntske/1`, and
/// the roots that sign the servers' certificates.
pub(crate) fn tls(roots: RootCertStore) -> Arc<ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("ring supports TLS 1.3")
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = vec![ALPN.to_vec()];
    Arc::new(config)
}

/// Mozilla's roots, which the public NTS servers' certificates chain to.
pub(crate) fn public_roots() -> RootCertStore {
    RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    }
}

/// What a key exchange agreed: the keys, the cookies, and the NTP server
/// to ask, as `host:port`.
pub(crate) struct Association {
    c2s: Aes128SivAead,
    s2c: Aes128SivAead,
    pub(crate) cookies: Vec<Vec<u8>>,
    pub(crate) server: String,
}

impl std::fmt::Debug for Association {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Association")
            .field("cookies", &self.cookies.len())
            .field("server", &self.server)
            .finish_non_exhaustive()
    }
}

fn nts(reason: &str) -> ClockError {
    ClockError::Nts(reason.to_owned())
}

/// One key exchange record: the critical bit and type, then the body.
fn record(kind: u16, body: &[u8]) -> Vec<u8> {
    let length = u16::try_from(body.len()).expect("a record body fits 64 KiB");
    let mut record = Vec::with_capacity(4 + body.len());
    record.extend_from_slice(&kind.to_be_bytes());
    record.extend_from_slice(&length.to_be_bytes());
    record.extend_from_slice(body);
    record
}

/// The client's request: NTPv4, AES-SIV, end.
pub(crate) fn ke_request() -> Vec<u8> {
    [
        record(CRITICAL + NEXT_PROTOCOL, &NTPV4.to_be_bytes()),
        record(AEAD, &AES_SIV.to_be_bytes()),
        // The end record is type 0, so its type is the critical bit alone.
        record(CRITICAL, &[]),
    ]
    .concat()
}

/// What a server's key exchange response says.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Negotiated {
    pub(crate) cookies: Vec<Vec<u8>>,
    pub(crate) server: Option<String>,
    pub(crate) port: Option<u16>,
}

/// Reads key exchange records from `stream` until the end record, keeping
/// at most eight cookies, and refusing a response past 64 KiB.
pub(crate) fn ke_response(stream: &mut impl Read) -> Result<Negotiated, ClockError> {
    let mut negotiated = Negotiated::default();
    let (mut protocol, mut aead) = (false, false);
    let mut read = 0;
    loop {
        let mut header = [0; 4];
        stream.read_exact(&mut header)?;
        let kind = u16::from_be_bytes([header[0], header[1]]);
        let mut body = vec![0; usize::from(u16::from_be_bytes([header[2], header[3]]))];
        read += header.len() + body.len();
        if read > KE_LIMIT {
            return Err(nts("the key exchange response is too long"));
        }
        stream.read_exact(&mut body)?;
        let pairs = || {
            body.as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_be_bytes(*pair))
        };
        match kind & !CRITICAL {
            END => break,
            NEXT_PROTOCOL => protocol = pairs().eq([NTPV4]),
            AEAD => aead = pairs().eq([AES_SIV]),
            ERROR => return Err(nts("the key exchange server refused")),
            NEW_COOKIE => {
                if negotiated.cookies.len() < COOKIES {
                    negotiated.cookies.push(body);
                }
            }
            SERVER => {
                let host = String::from_utf8(body).map_err(|_| nts("a server name isn't text"))?;
                negotiated.server = Some(host);
            }
            PORT => negotiated.port = pairs().next(),
            WARNING => {}
            _ if kind & CRITICAL != 0 => return Err(nts("a critical record isn't known")),
            _ => {}
        }
    }
    if !protocol || !aead {
        return Err(nts("the server didn't agree NTPv4 with AES-SIV"));
    }
    if negotiated.cookies.is_empty() {
        return Err(nts("the server gave no cookies"));
    }
    Ok(negotiated)
}

/// The key a direction of an association uses, exported from the TLS
/// session: 0 for client to server, 1 for server to client.
pub(crate) fn exported(
    export: impl FnOnce(&mut [u8; 32], &[u8], &[u8]) -> Result<(), rustls::Error>,
    direction: u8,
) -> Result<Aes128SivAead, ClockError> {
    let context = [0, 0, 0, 15, direction];
    let mut key = [0; 32];
    export(&mut key, EXPORTER, &context).map_err(|failed| ClockError::Nts(failed.to_string()))?;
    Ok(Aes128SivAead::new(&Key::<Aes128SivAead>::from(key)))
}

/// A socket that gives up at a deadline, however the bytes trickle. TLS
/// reads and writes it as often as a record needs, and every one of those
/// gets only the time left.
struct Deadline {
    socket: TcpStream,
    started: Instant,
}

impl Deadline {
    /// Sets the socket's timeouts to the time left, or fails once none is.
    fn remaining(&self) -> io::Result<()> {
        let left = KE_DEADLINE.saturating_sub(self.started.elapsed());
        if left.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the key exchange took too long",
            ));
        }
        self.socket.set_read_timeout(Some(left))?;
        self.socket.set_write_timeout(Some(left))
    }
}

impl Read for Deadline {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.remaining()?;
        self.socket.read(buffer)
    }
}

impl Write for Deadline {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.remaining()?;
        self.socket.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.remaining()?;
        self.socket.flush()
    }
}

/// Connects to each of `addresses` in turn until one accepts, by
/// `deadline`.
fn connect(addresses: &[SocketAddr], deadline: Instant) -> Result<TcpStream, ClockError> {
    let mut failed = ClockError::NoServers;
    for address in addresses {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        match TcpStream::connect_timeout(address, left) {
            Ok(socket) => return Ok(socket),
            Err(error) => failed = error.into(),
        }
    }
    Err(failed)
}

/// Runs a key exchange with `host`, on port 4460, giving up after
/// [`KE_DEADLINE`], resolving and connecting included.
pub(crate) fn exchange(
    host: &str,
    resolver: &Arc<dyn Resolve>,
    tls: &Arc<ClientConfig>,
) -> Result<Association, ClockError> {
    let started = Instant::now();
    let deadline = started + KE_DEADLINE;
    let addresses = resolve_by(resolver, &format!("{host}:{KE_PORT}"), deadline)?;
    let socket = connect(&addresses, deadline)?;
    let name = ServerName::try_from(host.to_owned()).map_err(|_| nts("a host name isn't valid"))?;
    let connection = ClientConnection::new(tls.clone(), name)
        .map_err(|failed| ClockError::Nts(failed.to_string()))?;
    let mut stream = StreamOwned::new(connection, Deadline { socket, started });
    stream.write_all(&ke_request())?;
    stream.flush()?;
    let negotiated = ke_response(&mut stream)?;
    let export = |direction| {
        exported(
            |key, label, context| {
                stream
                    .conn
                    .export_keying_material(key, label, Some(context))
                    .map(|_| ())
            },
            direction,
        )
    };
    let (c2s, s2c) = (export(0)?, export(1)?);
    let server = negotiated.server.unwrap_or_else(|| host.to_owned());
    let port = negotiated.port.unwrap_or(123);
    Ok(Association {
        c2s,
        s2c,
        cookies: negotiated.cookies,
        server: format!("{server}:{port}"),
    })
}

/// An NTP extension field: type, length, and the body padded to a
/// multiple of four bytes.
pub(crate) fn field(kind: u16, body: &[u8]) -> Vec<u8> {
    let padded = body.len().next_multiple_of(4);
    let length = u16::try_from(4 + padded).expect("a field fits 64 KiB");
    let mut field = Vec::with_capacity(4 + padded);
    field.extend_from_slice(&kind.to_be_bytes());
    field.extend_from_slice(&length.to_be_bytes());
    field.extend_from_slice(body);
    field.resize(4 + padded, 0);
    field
}

/// The extension fields in `bytes`, with where each starts, or `None` when
/// one is malformed.
pub(crate) fn fields(bytes: &[u8]) -> Option<Vec<(usize, u16, &[u8])>> {
    let mut fields = Vec::new();
    let mut rest = bytes;
    while let Some(header) = rest.first_chunk::<4>() {
        let kind = u16::from_be_bytes([header[0], header[1]]);
        let length = usize::from(u16::from_be_bytes([header[2], header[3]]));
        if length < 4 || length % 4 != 0 {
            return None;
        }
        let (field, after) = rest.split_at_checked(length)?;
        fields.push((bytes.len() - rest.len(), kind, &field[4..]));
        rest = after;
    }
    rest.is_empty().then_some(fields)
}

/// The authenticator field over everything before it: the nonce, and the
/// AEAD's output for `plaintext`.
pub(crate) fn authenticator(
    cipher: &Aes128SivAead,
    nonce: [u8; 16],
    before: &[u8],
    plaintext: &[u8],
) -> Vec<u8> {
    let sealed = cipher
        .encrypt(
            &Nonce::<Aes128SivAead>::from(nonce),
            Payload {
                msg: plaintext,
                aad: before,
            },
        )
        .expect("AES-SIV seals any message");
    let mut body = Vec::new();
    body.extend_from_slice(&16u16.to_be_bytes());
    body.extend_from_slice(
        &u16::try_from(sealed.len())
            .expect("a seal fits")
            .to_be_bytes(),
    );
    body.extend_from_slice(&nonce);
    body.extend_from_slice(&sealed);
    field(AUTHENTICATOR, &body)
}

/// Opens an authenticator field's body over `before`, giving the
/// plaintext, or `None` when it was forged or altered.
pub(crate) fn open(cipher: &Aes128SivAead, body: &[u8], before: &[u8]) -> Option<Vec<u8>> {
    let nonce_length = usize::from(u16::from_be_bytes([*body.first()?, *body.get(1)?]));
    let sealed_length = usize::from(u16::from_be_bytes([*body.get(2)?, *body.get(3)?]));
    let nonce = body.get(4..4 + nonce_length)?;
    let sealed_at = 4 + nonce_length.next_multiple_of(4);
    let sealed = body.get(sealed_at..sealed_at + sealed_length)?;
    let nonce = Nonce::<Aes128SivAead>::try_from(nonce).ok()?;
    cipher
        .decrypt(
            &nonce,
            Payload {
                msg: sealed,
                aad: before,
            },
        )
        .ok()
}

/// Random bytes for an identifier or a nonce.
pub(crate) fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).expect("the system has randomness");
    bytes
}

impl Association {
    /// A request carrying one cookie, a unique identifier, and placeholders
    /// for the cookies that would refill the client's eight. `None` when
    /// no cookie is left.
    pub(crate) fn request(&mut self, header: [u8; 48], unique: [u8; 32]) -> Option<Vec<u8>> {
        let cookie = self.cookies.pop()?;
        let mut packet = header.to_vec();
        packet.extend(field(UNIQUE_ID, &unique));
        packet.extend(field(COOKIE, &cookie));
        for _ in self.cookies.len() + 1..COOKIES {
            packet.extend(field(PLACEHOLDER, &vec![0; cookie.len()]));
        }
        let sealed = authenticator(&self.c2s, random(), &packet, &[]);
        packet.extend(sealed);
        Some(packet)
    }

    /// Checks that `reply` answers the request with `unique`, authenticated
    /// by the server, and keeps the cookies it carries. Gives the NTP
    /// header.
    pub(crate) fn verify<'a>(&mut self, reply: &'a [u8], unique: &[u8; 32]) -> Option<&'a [u8]> {
        let (header, extensions) = reply.split_at_checked(48)?;
        let found = fields(extensions)?;
        let (at, _, body) = *found.iter().find(|(_, kind, _)| *kind == AUTHENTICATOR)?;
        // Only fields before the authenticator are authenticated.
        let echoed = found
            .iter()
            .take_while(|&&(_, kind, _)| kind != AUTHENTICATOR)
            .any(|&(_, kind, body)| kind == UNIQUE_ID && body == unique);
        if !echoed {
            return None;
        }
        let plaintext = open(&self.s2c, body, &reply[..48 + at])?;
        for (_, kind, cookie) in fields(&plaintext)? {
            if kind == COOKIE {
                self.cookies.push(cookie.to_vec());
            }
        }
        // A server that sends more than were asked for doesn't grow the
        // store past eight.
        self.cookies.truncate(COOKIES);
        Some(header)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cipher(byte: u8) -> Aes128SivAead {
        Aes128SivAead::new(&Key::<Aes128SivAead>::from([byte; 32]))
    }

    fn association() -> Association {
        Association {
            c2s: cipher(1),
            s2c: cipher(2),
            cookies: vec![vec![7; 8], vec![9; 8]],
            server: "ntp.test:123".to_owned(),
        }
    }

    #[test]
    fn a_request_asks_for_ntpv4_with_aes_siv() {
        assert_eq!(
            ke_request(),
            [0x80, 1, 0, 2, 0, 0, 0, 4, 0, 2, 0, 15, 0x80, 0, 0, 0]
        );
    }

    fn response(records: &[Vec<u8>]) -> Result<Negotiated, ClockError> {
        ke_response(&mut records.concat().as_slice())
    }

    fn agreed() -> Vec<Vec<u8>> {
        vec![
            record(CRITICAL | NEXT_PROTOCOL, &[0, 0]),
            record(AEAD, &[0, 15]),
        ]
    }

    #[test]
    fn a_response_gives_cookies_and_the_server_to_ask() {
        let mut records = agreed();
        records.extend([
            record(CRITICAL + WARNING, &[0, 1]),
            record(99, b"ignored"),
            record(NEW_COOKIE, b"one"),
            record(NEW_COOKIE, b"two"),
            record(SERVER, b"ntp.test"),
            record(PORT, &[0x30, 0x39]),
            record(CRITICAL | END, &[]),
        ]);
        assert_eq!(
            response(&records).unwrap(),
            Negotiated {
                cookies: vec![b"one".to_vec(), b"two".to_vec()],
                server: Some("ntp.test".to_owned()),
                port: Some(12_345),
            }
        );
    }

    #[test]
    fn a_response_that_doesnt_agree_is_refused() {
        let end = record(CRITICAL | END, &[]);
        let cookie = record(NEW_COOKIE, b"c");
        let refused = |records: Vec<Vec<u8>>, why: &str| match response(&records) {
            Err(ClockError::Nts(reason)) => assert_eq!(reason, why),
            other => panic!("{other:?}"),
        };
        let disagreed = "the server didn't agree NTPv4 with AES-SIV";
        refused(
            vec![record(AEAD, &[0, 15]), cookie.clone(), end.clone()],
            disagreed,
        );
        refused(
            vec![record(NEXT_PROTOCOL, &[0, 0]), cookie.clone(), end.clone()],
            disagreed,
        );
        let other_aead = vec![
            record(NEXT_PROTOCOL, &[0, 0]),
            record(AEAD, &[0, 16]),
            cookie.clone(),
            end.clone(),
        ];
        refused(other_aead, disagreed);
        refused(
            [agreed(), vec![end.clone()]].concat(),
            "the server gave no cookies",
        );
        refused(
            vec![record(CRITICAL | ERROR, &[0, 1])],
            "the key exchange server refused",
        );
        refused(
            vec![record(CRITICAL | 99, &[])],
            "a critical record isn't known",
        );
        refused(vec![record(SERVER, &[0xff])], "a server name isn't text");
        assert!(matches!(
            response(&[record(NEW_COOKIE, b"c")]),
            Err(ClockError::Io(_))
        ));
    }

    #[test]
    fn a_response_keeps_eight_cookies_and_stops_at_64_kib() {
        let mut records = agreed();
        records.extend((0..12).map(|n| record(NEW_COOKIE, &[n])));
        records.push(record(CRITICAL | END, &[]));
        let negotiated = response(&records).unwrap();
        assert_eq!(
            negotiated.cookies,
            (0..8).map(|n| vec![n]).collect::<Vec<_>>()
        );
        // A 4-byte header and 1,020-byte body is 1,024 bytes a record.
        let mut flood = agreed();
        flood.extend((0..64).map(|_| record(NEW_COOKIE, &[0; 1_020])));
        flood.push(record(CRITICAL | END, &[]));
        match response(&flood) {
            Err(ClockError::Nts(reason)) => {
                assert_eq!(reason, "the key exchange response is too long");
            }
            other => panic!("{other:?}"),
        }
        // Exactly 64 KiB is read.
        let mut full = agreed();
        let used = full.iter().map(Vec::len).sum::<usize>() + 4;
        full.extend((0..63).map(|_| record(NEW_COOKIE, &[0; 1_020])));
        full.push(record(NEW_COOKIE, &vec![0; 1_024 - used - 4]));
        full.push(record(CRITICAL | END, &[]));
        assert_eq!(full.iter().map(Vec::len).sum::<usize>(), KE_LIMIT);
        assert!(response(&full).is_ok());
    }

    #[test]
    fn keys_are_exported_for_each_direction() {
        let seen = std::cell::RefCell::new(Vec::new());
        let export = |key: &mut [u8; 32], label: &[u8], context: &[u8]| {
            seen.borrow_mut().push((label.to_vec(), context.to_vec()));
            key.fill(context[4]);
            Ok(())
        };
        let to_server = exported(export, 0).unwrap();
        let to_client = exported(export, 1).unwrap();
        assert_eq!(
            *seen.borrow(),
            [
                (EXPORTER.to_vec(), vec![0, 0, 0, 15, 0]),
                (EXPORTER.to_vec(), vec![0, 0, 0, 15, 1]),
            ]
        );
        let sealed = authenticator(&to_server, [3; 16], b"ad", b"");
        assert!(open(&cipher(0), &sealed[4..], b"ad").is_some());
        assert!(open(&to_client, &sealed[4..], b"ad").is_none());
        let failed = exported(|_, _, _| Err(rustls::Error::HandshakeNotComplete), 0);
        assert!(matches!(failed, Err(ClockError::Nts(_))));
    }

    #[test]
    fn an_association_shows_its_cookies_and_server_but_not_its_keys() {
        assert_eq!(
            format!("{:?}", association()),
            r#"Association { cookies: 2, server: "ntp.test:123", .. }"#
        );
    }

    #[test]
    fn a_field_is_padded_to_four_bytes() {
        assert_eq!(field(0x0104, &[1, 2, 3]), [1, 4, 0, 8, 1, 2, 3, 0]);
        assert_eq!(field(0x0204, &[1, 2, 3, 4]), [2, 4, 0, 8, 1, 2, 3, 4]);
        assert_eq!(field(0x0304, &[]), [3, 4, 0, 4]);
    }

    #[test]
    fn fields_are_read_until_one_is_malformed() {
        let bytes = [field(1, &[5; 4]), field(2, &[])].concat();
        assert_eq!(
            fields(&bytes),
            Some(vec![(0, 1, &[5u8; 4][..]), (8, 2, &[][..])])
        );
        assert_eq!(fields(&[]), Some(vec![]));
        assert_eq!(fields(&[0, 1, 0, 3]), None);
        assert_eq!(fields(&[0, 1, 0, 6, 0, 0]), None);
        assert_eq!(fields(&[0, 1, 0, 12, 0, 0, 0, 0]), None);
        assert_eq!(fields(&[0, 1, 0]), None);
    }

    #[test]
    fn a_sealed_message_opens_only_unaltered() {
        let sealed = authenticator(&cipher(1), [3; 16], b"before", b"secret");
        let body = &sealed[4..];
        assert_eq!(open(&cipher(1), body, b"before"), Some(b"secret".to_vec()));
        assert_eq!(open(&cipher(1), body, b"altered"), None);
        assert_eq!(open(&cipher(2), body, b"before"), None);
        // The sealed bytes start after the lengths and the 16-byte nonce.
        let mut flipped = body.to_vec();
        flipped[20] ^= 1;
        assert_eq!(open(&cipher(1), &flipped, b"before"), None);
        assert_eq!(open(&cipher(1), &body[..3], b"before"), None);
        let short_nonce = [0, 8, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(open(&cipher(1), &short_nonce, b""), None);
    }

    #[test]
    fn a_request_spends_a_cookie_and_asks_for_the_rest() {
        let mut association = association();
        let packet = association.request([0; 48], [4; 32]).unwrap();
        assert_eq!(association.cookies, [vec![7; 8]]);
        let fields = fields(&packet[48..]).unwrap();
        let kinds: Vec<_> = fields.iter().map(|&(_, kind, _)| kind).collect();
        // One cookie left, so six placeholders refill eight.
        let mut expected = vec![UNIQUE_ID, COOKIE];
        expected.extend([PLACEHOLDER; 6]);
        expected.push(AUTHENTICATOR);
        assert_eq!(kinds, expected);
        assert_eq!(fields[1].2, [9; 8]);
        assert_eq!(fields[2].2, [0; 8]);
        let (at, _, body) = fields[8];
        assert_eq!(open(&cipher(1), body, &packet[..48 + at]), Some(vec![]));
        association.request([0; 48], [4; 32]).unwrap();
        assert!(association.request([0; 48], [4; 32]).is_none());
    }

    /// A server's reply: the header, the echoed identifier, and an
    /// authenticator sealing `cookies` with `key`.
    fn reply(unique: [u8; 32], key: &Aes128SivAead, cookies: &[&[u8]]) -> Vec<u8> {
        let mut packet = vec![0x24; 48];
        packet.extend(field(UNIQUE_ID, &unique));
        let plaintext: Vec<u8> = cookies
            .iter()
            .flat_map(|cookie| field(COOKIE, cookie))
            .collect();
        let sealed = authenticator(key, [5; 16], &packet, &plaintext);
        packet.extend(sealed);
        packet
    }

    #[test]
    fn a_verified_reply_gives_its_header_and_its_cookies() {
        let mut association = association();
        let packet = reply([4; 32], &cipher(2), &[b"new1", b"new2"]);
        assert_eq!(association.verify(&packet, &[4; 32]), Some(&packet[..48]));
        assert_eq!(association.cookies.len(), 4);
        assert_eq!(association.cookies[3], b"new2");
    }

    #[test]
    fn a_reply_with_more_cookies_than_asked_keeps_eight() {
        let mut association = association();
        let cookies: Vec<[u8; 4]> = (0..20).map(|n| [n; 4]).collect();
        let cookies: Vec<&[u8]> = cookies.iter().map(|cookie| &cookie[..]).collect();
        let packet = reply([4; 32], &cipher(2), &cookies);
        assert!(association.verify(&packet, &[4; 32]).is_some());
        assert_eq!(association.cookies.len(), 8);
    }

    #[test]
    fn a_reply_that_isnt_ours_or_isnt_authentic_is_refused() {
        let mut association = association();
        let refused = |packet: &[u8], association: &mut Association| {
            assert_eq!(association.verify(packet, &[4; 32]), None);
            assert_eq!(association.cookies.len(), 2);
        };
        refused(&reply([6; 32], &cipher(2), &[b"c"]), &mut association);
        refused(&reply([4; 32], &cipher(1), &[b"c"]), &mut association);
        let mut altered = reply([4; 32], &cipher(2), &[b"c"]);
        altered[10] ^= 1;
        refused(&altered, &mut association);
        refused(&[0x24; 48], &mut association);
        refused(&[0x24; 47], &mut association);
        let mut malformed = vec![0x24; 48];
        malformed.extend([0, 1, 0, 3]);
        refused(&malformed, &mut association);
        // The identifier must come before the authenticator, not after.
        let mut late = vec![0x24; 48];
        let sealed = authenticator(&cipher(2), [5; 16], &late, &[]);
        late.extend(sealed);
        late.extend(field(UNIQUE_ID, &[4; 32]));
        refused(&late, &mut association);
        // Plaintext that isn't fields gives no cookies.
        let mut garbled = vec![0x24; 48];
        garbled.extend(field(UNIQUE_ID, &[4; 32]));
        let sealed = authenticator(&cipher(2), [5; 16], &garbled, &[0, 1, 0, 3]);
        garbled.extend(sealed);
        refused(&garbled, &mut association);
    }

    #[test]
    fn random_bytes_differ() {
        assert_ne!(random::<16>(), random::<16>());
    }
}
