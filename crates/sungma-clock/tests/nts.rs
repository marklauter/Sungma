//! NTS end to end: key exchange over TLS 1.3, then authenticated NTP,
//! against servers simulated on the loopback interface.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, UdpSocket},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use aes_siv::{
    Aes128SivAead, KeyInit,
    aead::{Aead, Key, Nonce, Payload},
};
use rustls::{
    ServerConfig, ServerConnection, StreamOwned,
    pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer},
};
use sungma::clock::Clock;
use sungma_clock::{ManualTime, NtpClock, Servers};

const EXPORTER: &[u8] = b"EXPORTER-network-time-security";
const UNIQUE_ID: u16 = 0x0104;
const COOKIE: u16 = 0x0204;
const PLACEHOLDER: u16 = 0x0304;
const AUTHENTICATOR: u16 = 0x0404;

/// What a server does with the next request.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Behavior {
    Honest,
    /// Alters a byte after sealing.
    Tamper,
    /// Seals with the client's key instead of its own.
    WrongKey,
    /// Answers with an unauthenticated NTS NAK.
    Nak,
    /// Answers with a NAK that doesn't echo the request's identifier.
    NakWithoutId,
    /// Sends a stray packet, then an honest reply.
    Stray,
    /// Answers with an authenticated Kiss-o'-Death DENY.
    Deny,
}

#[derive(Default)]
struct State {
    /// The client-to-server and server-to-client keys of the last exchange.
    keys: Option<([u8; 32], [u8; 32])>,
    issued: HashSet<Vec<u8>>,
    spent: Vec<Vec<u8>>,
    exchanges: usize,
    requests: usize,
    behaviors: VecDeque<Behavior>,
}

/// How a server runs its key exchange.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Exchange {
    Honest,
    /// Trickles its response a byte every 50 ms.
    Slow,
    /// Trickles every TLS byte, the handshake's included, one every 20 ms,
    /// so no TLS record is ever whole for long.
    Trickle,
    /// Streams more than 64 KiB of cookies.
    Flood,
}

/// A socket that writes one byte at a time, `delay` apart, when it has a
/// delay.
struct Trickling {
    socket: std::net::TcpStream,
    delay: Option<Duration>,
}

impl Read for Trickling {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.socket.read(buffer)
    }
}

impl Write for Trickling {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let Some(delay) = self.delay else {
            return self.socket.write(buffer);
        };
        thread::sleep(delay);
        self.socket.write(&buffer[..buffer.len().min(1)])
    }

    fn flush(&mut self) -> io::Result<()> {
        self.socket.flush()
    }
}

/// An NTS server: a key exchange listener and an NTP socket.
struct NtsServer {
    host: &'static str,
    ke: SocketAddr,
    state: Arc<Mutex<State>>,
}

impl NtsServer {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }
}

fn cipher(key: [u8; 32]) -> Aes128SivAead {
    Aes128SivAead::new(&Key::<Aes128SivAead>::from(key))
}

fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).unwrap();
    bytes
}

fn field(kind: u16, body: &[u8]) -> Vec<u8> {
    let padded = body.len().next_multiple_of(4);
    let mut field = kind.to_be_bytes().to_vec();
    field.extend_from_slice(&u16::try_from(4 + padded).unwrap().to_be_bytes());
    field.extend_from_slice(body);
    field.resize(4 + padded, 0);
    field
}

fn fields(bytes: &[u8]) -> Vec<(usize, u16, &[u8])> {
    let mut fields = Vec::new();
    let mut at = 0;
    while at + 4 <= bytes.len() {
        let kind = u16::from_be_bytes([bytes[at], bytes[at + 1]]);
        let length = usize::from(u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]));
        fields.push((at, kind, &bytes[at + 4..at + length]));
        at += length;
    }
    fields
}

fn seal(key: [u8; 32], before: &[u8], plaintext: &[u8]) -> Vec<u8> {
    let nonce: [u8; 16] = random();
    let sealed = cipher(key)
        .encrypt(
            &Nonce::<Aes128SivAead>::from(nonce),
            Payload {
                msg: plaintext,
                aad: before,
            },
        )
        .unwrap();
    let mut body = 16u16.to_be_bytes().to_vec();
    body.extend_from_slice(&u16::try_from(sealed.len()).unwrap().to_be_bytes());
    body.extend_from_slice(&nonce);
    body.extend_from_slice(&sealed);
    field(AUTHENTICATOR, &body)
}

fn opens(key: [u8; 32], body: &[u8], before: &[u8]) -> bool {
    let sealed_length = usize::from(u16::from_be_bytes([body[2], body[3]]));
    let nonce = Nonce::<Aes128SivAead>::try_from(&body[4..20]).unwrap();
    let payload = Payload {
        msg: &body[20..20 + sealed_length],
        aad: before,
    };
    cipher(key).decrypt(&nonce, payload).is_ok()
}

fn record(kind: u16, body: &[u8]) -> Vec<u8> {
    let mut record = kind.to_be_bytes().to_vec();
    record.extend_from_slice(&u16::try_from(body.len()).unwrap().to_be_bytes());
    record.extend_from_slice(body);
    record
}

/// A certificate for every simulated host.
fn certificate() -> rcgen::CertifiedKey<rcgen::KeyPair> {
    let hosts = ["a.test", "b.test", "c.test", "d.test"].map(str::to_owned);
    rcgen::generate_simple_self_signed(hosts.to_vec()).unwrap()
}

fn tls(certified: &rcgen::CertifiedKey<rcgen::KeyPair>) -> Arc<ServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let key = PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der());
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![certified.cert.der().clone()],
            PrivateKeyDer::Pkcs8(key),
        )
        .unwrap();
    config.alpn_protocols = vec![b"ntske/1".to_vec()];
    Arc::new(config)
}

/// Starts a server for `host` that behaves as `behaviors` says, request by
/// request, and honestly after them.
fn serve(host: &'static str, config: Arc<ServerConfig>, behaviors: &[Behavior]) -> NtsServer {
    serve_exchanging(host, config, behaviors, Exchange::Honest)
}

fn serve_exchanging(
    host: &'static str,
    config: Arc<ServerConfig>,
    behaviors: &[Behavior],
    exchange: Exchange,
) -> NtsServer {
    let state = Arc::new(Mutex::new(State {
        behaviors: behaviors.iter().copied().collect(),
        ..State::default()
    }));
    let ntp = UdpSocket::bind("127.0.0.1:0").unwrap();
    let ntp_port = ntp.local_addr().unwrap().port();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let ke = listener.local_addr().unwrap();
    let shared = state.clone();
    thread::spawn(move || {
        for tcp in listener.incoming() {
            let connection = ServerConnection::new(config.clone()).unwrap();
            let delay = (exchange == Exchange::Trickle).then_some(Duration::from_millis(20));
            let mut stream = StreamOwned::new(
                connection,
                Trickling {
                    socket: tcp.unwrap(),
                    delay,
                },
            );
            // The client's records end with the critical end record.
            // A client that refuses the certificate never sends them.
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(&[0x80, 0, 0, 0]) && stream.read_exact(&mut byte).is_ok() {
                request.push(byte[0]);
            }
            if !request.ends_with(&[0x80, 0, 0, 0]) {
                continue;
            }
            let export = |direction| {
                let context = [0, 0, 0, 15, direction];
                stream
                    .conn
                    .export_keying_material([0; 32], EXPORTER, Some(&context))
                    .unwrap()
            };
            let keys = (export(0), export(1));
            let cookies: Vec<Vec<u8>> = (0..8).map(|_| random::<64>().to_vec()).collect();
            {
                let mut state = shared.lock().unwrap();
                state.keys = Some(keys);
                state.issued.extend(cookies.iter().cloned());
                state.exchanges += 1;
            }
            let mut response = [
                record(0x8001, &[0, 0]),
                record(4, &[0, 15]),
                record(6, b"127.0.0.1"),
                record(7, &ntp_port.to_be_bytes()),
            ]
            .concat();
            for cookie in &cookies {
                response.extend(record(5, cookie));
            }
            if exchange == Exchange::Flood {
                for _ in 0..80 {
                    response.extend(record(5, &[0; 1_020]));
                }
            }
            response.extend(record(0x8000, &[]));
            if exchange == Exchange::Slow {
                for byte in response {
                    if stream
                        .write_all(&[byte])
                        .and_then(|()| stream.flush())
                        .is_err()
                    {
                        break;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                continue;
            }
            let _ = stream.write_all(&response);
            stream.conn.send_close_notify();
            let _ = stream.flush();
        }
    });
    let shared = state.clone();
    thread::spawn(move || {
        loop {
            let mut buffer = [0; 2048];
            let (length, client) = ntp.recv_from(&mut buffer).unwrap();
            let request = &buffer[..length];
            let mut state = shared.lock().unwrap();
            state.requests += 1;
            let behavior = state.behaviors.pop_front().unwrap_or(Behavior::Honest);
            let (to_server, to_client) = state.keys.unwrap();
            let found = fields(&request[48..]);
            let find = |kind| found.iter().find(|field| field.1 == kind).copied();
            let (_, _, unique) = find(UNIQUE_ID).unwrap();
            let (_, _, cookie) = find(COOKIE).unwrap();
            let (at, _, body) = find(AUTHENTICATOR).unwrap();
            let placeholders = found.iter().filter(|field| field.1 == PLACEHOLDER).count();
            // A request must be authentic and spend a cookie only once.
            if !opens(to_server, body, &request[..48 + at]) || !state.issued.remove(cookie) {
                continue;
            }
            state.spent.push(cookie.to_vec());
            let origin = &request[40..48];
            let mut header = [0; 48];
            header[0] = 0x24;
            header[1] = 2;
            header[8..12].copy_from_slice(&655u32.to_be_bytes());
            for at in [24, 32, 40] {
                header[at..at + 8].copy_from_slice(origin);
            }
            if matches!(
                behavior,
                Behavior::Nak | Behavior::NakWithoutId | Behavior::Deny
            ) {
                header[0] = 0xe4;
                header[1] = 0;
                let code = if behavior == Behavior::Deny {
                    b"DENY"
                } else {
                    b"NTSN"
                };
                header[12..16].copy_from_slice(code);
            }
            match behavior {
                Behavior::Nak => {
                    let nak = [header.to_vec(), field(UNIQUE_ID, unique)].concat();
                    ntp.send_to(&nak, client).unwrap();
                    continue;
                }
                Behavior::NakWithoutId => {
                    ntp.send_to(&header, client).unwrap();
                    continue;
                }
                Behavior::Stray => {
                    ntp.send_to(&[0x24; 48], client).unwrap();
                }
                _ => {}
            }
            let mut packet = header.to_vec();
            packet.extend(field(UNIQUE_ID, unique));
            let mut plaintext = Vec::new();
            for _ in 0..=placeholders {
                let cookie = random::<64>().to_vec();
                plaintext.extend(field(COOKIE, &cookie));
                state.issued.insert(cookie);
            }
            let key = if behavior == Behavior::WrongKey {
                to_server
            } else {
                to_client
            };
            let sealed = seal(key, &packet, &plaintext);
            packet.extend(sealed);
            if behavior == Behavior::Tamper {
                packet[44] ^= 1;
            }
            ntp.send_to(&packet, client).unwrap();
        }
    });
    NtsServer { host, ke, state }
}

/// NTS servers trusting only the test certificate, resolved to the
/// simulated hosts.
fn servers(nts: &[&NtsServer], certificate: &[u8]) -> Servers {
    let addresses: HashMap<String, SocketAddr> = nts
        .iter()
        .map(|server| (format!("{}:4460", server.host), server.ke))
        .collect();
    let hosts: Vec<_> = nts.iter().map(|server| server.host).collect();
    let refusing = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    Servers::nts(hosts)
        .trusting_only(certificate)
        .unwrap()
        .resolving_with(move |name: &str| match addresses.get(name) {
            // A refusing address first: the exchange must move on to the next.
            Some(address) => Ok(vec![refusing, *address]),
            None => name
                .parse()
                .map(|address| vec![address])
                .map_err(|failed| io::Error::new(io::ErrorKind::InvalidInput, failed)),
        })
}

fn wall() -> u64 {
    let since = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    u64::try_from(since.as_nanos()).unwrap()
}

/// Three honest servers and a fourth that behaves as `fourth` says.
fn four(fourth: &[Behavior]) -> (Vec<NtsServer>, Vec<u8>) {
    let certified = certificate();
    let config = tls(&certified);
    let servers = vec![
        serve("a.test", config.clone(), &[]),
        serve("b.test", config.clone(), &[]),
        serve("c.test", config.clone(), &[]),
        serve("d.test", config, fourth),
    ];
    (servers, certified.cert.der().to_vec())
}

fn all(nts: &[NtsServer]) -> Vec<&NtsServer> {
    nts.iter().collect()
}

#[test]
fn a_key_exchange_gives_cookies_and_an_authenticated_sync() {
    let (nts, certificate) = four(&[]);
    let clock = NtpClock::sync(servers(&all(&nts), &certificate)).unwrap();
    let (before, now, after) = (wall(), clock.now().unwrap(), wall());
    assert!(now.settled.0 <= after && before <= now.revision.0);
    for server in &nts {
        let state = server.state();
        assert_eq!((state.exchanges, state.requests), (1, 1));
    }
}

#[test]
fn a_cookie_is_spent_once_and_each_reply_replaces_it() {
    let (nts, certificate) = four(&[]);
    let clock = NtpClock::sync(servers(&all(&nts), &certificate)).unwrap();
    for _ in 0..9 {
        clock.resync().unwrap();
    }
    for server in &nts {
        let state = server.state();
        // Ten requests on eight cookies: replies refilled them, so the
        // keys were exchanged once.
        assert_eq!(state.exchanges, 1);
        let spent: HashSet<_> = state.spent.iter().collect();
        assert_eq!(spent.len(), 10);
    }
}

#[test]
fn an_altered_or_wrongly_sealed_reply_is_refused() {
    for behavior in [Behavior::Tamper, Behavior::WrongKey] {
        let (nts, certificate) = four(&[behavior]);
        let clock = NtpClock::sync(servers(&all(&nts), &certificate)).unwrap();
        assert_eq!(
            clock.last_sync().unanswered,
            vec!["d.test".to_owned()],
            "{behavior:?}"
        );
    }
}

#[test]
fn a_nak_exchanges_keys_again() {
    let (nts, certificate) = four(&[Behavior::Honest, Behavior::Nak]);
    let clock = NtpClock::sync(servers(&all(&nts), &certificate)).unwrap();
    clock.resync().unwrap();
    assert_eq!(clock.last_sync().unanswered, vec!["d.test".to_owned()]);
    clock.resync().unwrap();
    assert!(clock.last_sync().unanswered.is_empty());
    assert_eq!(nts[3].state().exchanges, 2);
}

#[test]
fn a_nak_without_our_identifier_is_ignored() {
    let (nts, certificate) = four(&[Behavior::Honest, Behavior::NakWithoutId]);
    let clock = NtpClock::sync(servers(&all(&nts), &certificate)).unwrap();
    clock.resync().unwrap();
    assert_eq!(clock.last_sync().unanswered, vec!["d.test".to_owned()]);
    clock.resync().unwrap();
    // Its cookies weren't dropped, so there was no second exchange.
    assert_eq!(nts[3].state().exchanges, 1);
}

#[test]
fn a_stray_packet_before_the_reply_is_skipped() {
    let (nts, certificate) = four(&[Behavior::Stray]);
    let clock = NtpClock::sync(servers(&all(&nts), &certificate)).unwrap();
    assert!(clock.last_sync().unanswered.is_empty());
}

/// Three honest servers and a fourth whose key exchange runs as
/// `exchange` says.
fn four_exchanging(exchange: Exchange) -> (Vec<NtsServer>, Vec<u8>) {
    let certified = certificate();
    let config = tls(&certified);
    let servers = vec![
        serve("a.test", config.clone(), &[]),
        serve("b.test", config.clone(), &[]),
        serve("c.test", config.clone(), &[]),
        serve_exchanging("d.test", config, &[], exchange),
    ];
    (servers, certified.cert.der().to_vec())
}

#[test]
fn a_key_exchange_that_trickles_gives_up_at_its_deadline() {
    let (nts, certificate) = four_exchanging(Exchange::Slow);
    let started = Instant::now();
    let clock = NtpClock::sync(servers(&all(&nts), &certificate)).unwrap();
    let took = started.elapsed();
    assert!(
        took >= Duration::from_secs(5) && took < Duration::from_secs(7),
        "{took:?}"
    );
    assert_eq!(clock.last_sync().unanswered, vec!["d.test".to_owned()]);
}

#[test]
fn a_key_exchange_that_trickles_its_tls_bytes_gives_up_at_its_deadline() {
    let (nts, certificate) = four_exchanging(Exchange::Trickle);
    let started = Instant::now();
    let clock = NtpClock::sync(servers(&all(&nts), &certificate)).unwrap();
    let took = started.elapsed();
    assert!(
        took >= Duration::from_secs(5) && took < Duration::from_secs(7),
        "{took:?}"
    );
    assert_eq!(clock.last_sync().unanswered, vec!["d.test".to_owned()]);
}

#[test]
fn a_key_exchange_that_floods_is_refused() {
    let (nts, certificate) = four_exchanging(Exchange::Flood);
    let clock = NtpClock::sync(servers(&all(&nts), &certificate)).unwrap();
    assert_eq!(clock.last_sync().unanswered, vec!["d.test".to_owned()]);
}

/// Real NTS servers on the internet, so a misreading of RFC 8915 that our
/// own server shares can't pass. Run with `cargo test -- --ignored`.
#[test]
#[ignore = "needs the internet"]
fn public_nts_servers_sync() {
    let hosts = [
        "time.cloudflare.com",
        "nts.netnod.se",
        "ptbtime1.ptb.de",
        "nts.time.nl",
    ];
    let clock = NtpClock::sync(Servers::nts(hosts)).unwrap();
    let report = clock.last_sync();
    let now = clock.now().unwrap();
    println!("{report:?}");
    println!("bound: {} ms", (now.revision.0 - now.settled.0) / 2_000_000);
    let (before, after) = (wall() - 2_000_000_000, wall() + 2_000_000_000);
    assert!(now.settled.0 <= after && before <= now.revision.0);
    clock.resync().unwrap();
    println!("{:?}", clock.last_sync());
}

#[test]
fn an_authenticated_deny_is_honored_for_good() {
    let (nts, certificate) = four(&[Behavior::Honest, Behavior::Deny]);
    let time = Arc::new(ManualTime::new(wall()));
    let clock = NtpClock::sync_with(servers(&all(&nts), &certificate), time.clone()).unwrap();
    clock.resync().unwrap();
    let report = clock.last_sync();
    assert_eq!(report.kissed, vec![("d.test".to_owned(), *b"DENY")]);
    time.advance(Duration::from_secs(100_000));
    clock.resync().unwrap();
    assert_eq!(nts[3].state().requests, 2);
    assert_eq!(clock.last_sync().denied, vec!["d.test".to_owned()]);
    assert!(clock.last_sync().overridden.is_empty());
}

#[test]
fn a_failed_key_exchange_leaves_the_server_unanswered() {
    let (nts, certificate) = four(&[]);
    let gone = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = gone.local_addr().unwrap();
    drop(gone);
    let mut list = all(&nts);
    let missing = NtsServer {
        host: "d.test",
        ke: address,
        state: Arc::default(),
    };
    list[3] = &missing;
    let clock = NtpClock::sync(servers(&list, &certificate)).unwrap();
    assert_eq!(clock.last_sync().unanswered, vec!["d.test".to_owned()]);
    let untrusted = certificate_of_another_authority();
    let refused = NtpClock::sync(servers(&all(&nts)[..3], &untrusted)).unwrap_err();
    assert!(refused.to_string().contains("certificate"), "{refused}");
}

fn certificate_of_another_authority() -> Vec<u8> {
    certificate().cert.der().to_vec()
}

#[test]
fn an_address_that_drops_packets_gives_the_next_its_share_of_the_deadline() {
    let (nts, certificate) = four(&[]);
    let blackhole: SocketAddr = "192.0.2.1:4460".parse().unwrap();
    let addresses: HashMap<String, SocketAddr> = nts
        .iter()
        .map(|server| (format!("{}:4460", server.host), server.ke))
        .collect();
    let hosts: Vec<_> = nts.iter().map(|server| server.host).collect();
    let servers = Servers::nts(hosts)
        .trusting_only(&certificate)
        .unwrap()
        .resolving_with(move |name: &str| match addresses.get(name) {
            Some(address) => Ok(vec![blackhole, *address]),
            None => name
                .parse()
                .map(|address| vec![address])
                .map_err(|failed| io::Error::new(io::ErrorKind::InvalidInput, failed)),
        });
    let clock = NtpClock::sync(servers).unwrap();
    assert!(clock.last_sync().unanswered.is_empty());
}
