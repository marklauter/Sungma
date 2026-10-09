//! An SNTP client, for when the platform reports no bound of its own.
//!
//! A sync asks every server at once. A reply gives the system clock's
//! offset from the server and a bound: half the round trip, plus half the
//! server's root delay, plus its root dispersion, which is RFC 5905's root
//! distance. The clock keeps the range where most servers agree, as
//! Marzullo's algorithm does. A server outside it is a falseticker. A sync
//! fails unless at least three servers answer and more than half of those
//! agree.

use std::{
    future::Future,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket},
    sync::{Mutex, PoisonError},
    thread,
    time::Duration,
};

use sungma::clock::{Clock, ClockFault, Reading};

use crate::{
    ClockError, OsTime, Resolve, Servers, TimeSource,
    nts::{self, Association},
    wall::{self, Sample, Sampled},
};

/// Seconds from 1900, NTP's epoch, to 1970, the Unix epoch.
const UNIX_EPOCH_SECONDS: i128 = 2_208_988_800;
const NANOS: i128 = 1_000_000_000;
const PACKET: usize = 48;
const TIMEOUT: Duration = Duration::from_secs(1);

/// The fewest answers a sync takes. Fewer can't outvote a falseticker, so
/// the sync fails, and the window's bound grows until a sync succeeds.
const MIN_ANSWERS: usize = 3;

/// How often a server is asked at most, NTP's shortest standard poll,
/// 64 s, which a Kiss-o'-Death `RATE` doubles.
const MIN_POLL: u64 = 64_000_000_000;

/// The longest a Kiss-o'-Death holds a server off, 1024 s, chrony's default
/// maxpoll. Kisses aren't authenticated, so an attacker who sees our
/// requests can send them; a short hold stops one mattering soon after the
/// attack does.
const MAX_HOLD: u64 = 1_024_000_000_000;

/// The longest an authenticated `RATE`, sent over NTS, stretches a
/// server's poll, NTP's maximum, 2^17 s.
const MAX_POLL: u64 = 131_072_000_000_000;

/// The largest reply read: room for an NTS reply's cookies.
const REPLY: usize = 2048;

/// A leap second a server announces for the end of the day.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Leap {
    Insert,
    Delete,
}

/// What the last sync found, whether it succeeded or not.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct SyncReport {
    /// Servers that answered outside the majority's range.
    pub falsetickers: Vec<String>,
    /// Servers that didn't answer, or whose answer was refused.
    pub unanswered: Vec<String>,
    /// Servers that sent Kiss-o'-Death this sync, with its code.
    pub kissed: Vec<(String, [u8; 4])>,
    /// Servers held off by a `DENY` or `RSTR`, until they answer again.
    pub denied: Vec<String>,
    /// Servers asked although a Kiss-o'-Death held them, because honoring
    /// every hold would leave too few to sync.
    pub overridden: Vec<String>,
    /// A leap second announced by a server in the majority.
    pub leap: Option<Leap>,
}

/// When a server may next be asked, after a Kiss-o'-Death.
#[derive(Clone, Copy, Debug)]
struct Schedule {
    poll: u64,
    /// The monotonic time before which the server isn't asked.
    held_until: u64,
    /// `DENY` or `RSTR`: the server asked not to be asked again. It is
    /// held for [`MAX_HOLD`], and the flag clears when it answers.
    denied: bool,
    /// The `DENY` or `RSTR` came authenticated over NTS, so it is honored
    /// as written: the server is never asked again.
    permanent: bool,
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            poll: MIN_POLL,
            held_until: 0,
            denied: false,
            permanent: false,
        }
    }
}

impl Schedule {
    /// Notes that the server was asked at monotonic time `now`. A server
    /// whose poll a `RATE` raised is held off for that poll after every
    /// ask, not only after the kiss.
    fn asked(&mut self, now: u64) {
        if self.poll > MIN_POLL {
            self.held_until = now.saturating_add(self.poll);
        }
    }

    /// Takes in a Kiss-o'-Death sent at monotonic time `now`. One that NTS
    /// `authenticated` is honored as written; any other holds the server
    /// for at most [`MAX_HOLD`], since an attacker could have sent it.
    fn kissed(&mut self, code: [u8; 4], now: u64, authenticated: bool) {
        match &code {
            b"RATE" => {
                let longest = if authenticated { MAX_POLL } else { MAX_HOLD };
                self.poll = self.poll.saturating_mul(2).min(longest);
                self.asked(now);
            }
            b"DENY" | b"RSTR" => {
                self.denied = true;
                self.permanent = authenticated;
                self.held_until = if authenticated {
                    u64::MAX
                } else {
                    now.saturating_add(MAX_HOLD)
                };
            }
            _ => {}
        }
    }

    fn askable(&self, now: u64) -> bool {
        now >= self.held_until
    }
}

/// The system clock, corrected by the servers' offset, give or take their
/// bound. Syncing blocks on the network for up to a second, so an async
/// caller runs it on a blocking thread.
#[derive(Debug)]
pub struct NtpClock<T = OsTime> {
    servers: Servers,
    peers: Mutex<Vec<Peer>>,
    last: Mutex<SyncReport>,
    sampled: Sampled<T>,
}

impl NtpClock {
    /// Takes a first sample from `servers`, on the operating system's
    /// clocks.
    pub fn sync(servers: Servers) -> Result<Self, ClockError> {
        Self::sync_with(servers, OsTime)
    }
}

impl<T: TimeSource> NtpClock<T> {
    /// [`NtpClock::sync`], on the clocks `time` reads.
    pub fn sync_with(servers: Servers, time: T) -> Result<Self, ClockError> {
        let mut peers: Vec<Peer> = servers.names.iter().map(|_| Peer::default()).collect();
        let (sample, report) = sync(&servers, &mut peers, &time);
        let sample = sample?;
        Ok(Self {
            servers,
            peers: Mutex::new(peers),
            last: Mutex::new(report),
            sampled: Sampled::new(sample, time),
        })
    }

    /// Takes a fresh sample into the window. When no server can be
    /// reached, the window stays as it was and its bounds keep growing.
    /// [`ClockFault::Drifted`] when the sample disagrees with the window,
    /// which the clock then replaces with the sample alone.
    pub fn resync(&self) -> Result<(), ClockError> {
        let mut peers = self.peers.lock().unwrap_or_else(PoisonError::into_inner);
        let (sample, report) = sync(&self.servers, &mut peers, self.sampled.time());
        *self.last.lock().unwrap_or_else(PoisonError::into_inner) = report;
        Ok(self.sampled.add(sample?)?)
    }

    /// What the last sync found, whether it succeeded or not.
    pub fn last_sync(&self) -> SyncReport {
        self.last
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl<T: TimeSource> Clock for NtpClock<T> {
    fn now(&self) -> Result<Reading, ClockFault> {
        self.sampled.now()
    }

    fn wait(&self, stamped: Reading) -> impl Future<Output = Result<(), ClockFault>> + Send {
        wall::wait(self, stamped)
    }
}

/// A server's schedule, and its NTS association once it has one.
#[derive(Debug, Default)]
struct Peer {
    schedule: Schedule,
    association: Option<Association>,
}

/// A server's reply, and whether NTS authenticated it.
#[derive(Debug)]
struct Reply {
    result: Result<Answer, ClockError>,
    authenticated: bool,
}

impl Reply {
    fn unauthenticated(result: Result<Answer, ClockError>) -> Self {
        Self {
            result,
            authenticated: false,
        }
    }
}

/// Asks every server that may be asked, at once, and keeps the majority's
/// range. The report says what the sync found, even when it fails.
fn sync(
    servers: &Servers,
    peers: &mut [Peer],
    time: &impl TimeSource,
) -> (Result<Sample, ClockError>, SyncReport) {
    let now = time.monotonic();
    let mut report = SyncReport::default();
    let mut asked: Vec<usize> = (0..servers.names.len())
        .filter(|&at| peers[at].schedule.askable(now))
        .collect();
    // Too few left to sync: ask the held servers whose holds end soonest,
    // as many as quorum needs. That is still no more than NTP's 64 s poll.
    // A server that denied us over NTS stays denied.
    let needed = MIN_ANSWERS.saturating_sub(asked.len());
    let mut held: Vec<usize> = (0..servers.names.len())
        .filter(|at| !asked.contains(at) && !peers[*at].schedule.permanent)
        .collect();
    held.sort_by_key(|&at| (peers[at].schedule.held_until, at));
    for at in held.into_iter().take(needed) {
        report.overridden.push(servers.names[at].clone());
        asked.push(at);
    }
    for &at in &asked {
        peers[at].schedule.asked(now);
    }
    let associations = asked
        .iter()
        .map(|&at| peers[at].association.take())
        .collect();
    let mut replies = Vec::new();
    for (&at, (reply, association)) in
        asked
            .iter()
            .zip(query_all(servers, &asked, associations, time))
    {
        peers[at].association = association;
        replies.push(reply);
    }
    let sample = agree(servers, peers, &asked, replies, now, &mut report);
    for (name, peer) in servers.names.iter().zip(peers.iter()) {
        if peer.schedule.denied {
            report.denied.push(name.clone());
        }
    }
    (sample, report)
}

/// Queries the servers at `asked`, each on its own thread, giving back each
/// NTS server's association.
fn query_all(
    servers: &Servers,
    asked: &[usize],
    associations: Vec<Option<Association>>,
    time: &impl TimeSource,
) -> Vec<(Reply, Option<Association>)> {
    thread::scope(|scope| {
        let queries: Vec<_> = asked
            .iter()
            .zip(associations)
            .map(|(&at, association)| {
                let name = &servers.names[at];
                scope.spawn(move || {
                    if servers.nts[at] {
                        query_nts(name, association, servers, time)
                    } else {
                        let resolved = servers.resolver.resolve(name);
                        let result = resolved
                            .map_err(ClockError::from)
                            .and_then(|address| query(address, time));
                        (Reply::unauthenticated(result), None)
                    }
                })
            })
            .collect();
        queries
            .into_iter()
            .map(|query| query.join().expect("a query doesn't panic"))
            .collect()
    })
}

/// Sorts the replies into the report, and keeps the majority's range.
fn agree(
    servers: &Servers,
    peers: &mut [Peer],
    asked: &[usize],
    replies: Vec<Reply>,
    now: u64,
    report: &mut SyncReport,
) -> Result<Sample, ClockError> {
    let mut answers = Vec::new();
    let mut error = ClockError::NoServers;
    for (&at, reply) in asked.iter().zip(replies) {
        let name = servers.names[at].clone();
        match reply.result {
            Ok(answer) => {
                peers[at].schedule.denied = false;
                answers.push((name, answer));
            }
            Err(ClockError::Kiss(code)) => {
                peers[at].schedule.kissed(code, now, reply.authenticated);
                report.kissed.push((name, code));
                error = ClockError::Kiss(code);
            }
            Err(failed) => {
                report.unanswered.push(name);
                error = failed;
            }
        }
    }
    if answers.is_empty() {
        return Err(error);
    }
    if answers.len() < MIN_ANSWERS {
        return Err(ClockError::TooFewAnswers {
            answered: answers.len(),
        });
    }
    let ranges: Vec<_> = answers
        .iter()
        .map(|(_, answer)| range(&answer.sample))
        .collect();
    let agreeing = majority(&ranges).ok_or(ClockError::Disagree)?;
    let mut samples = Vec::new();
    for (at, (name, answer)) in answers.into_iter().enumerate() {
        if agreeing.contains(&at) {
            samples.push(answer.sample);
            report.leap = report.leap.or(answer.leap);
        } else {
            report.falsetickers.push(name);
        }
    }
    intersect(&samples)
}

/// The range a sample allows, its low and high ends.
fn range(sample: &Sample) -> (i128, i128) {
    let offset = i128::from(sample.offset);
    let bound = i128::from(sample.bound);
    (offset - bound, offset + bound)
}

/// The ranges that share the point most of them share, by index, or `None`
/// unless more than half do. Of points shared by equally many, the
/// earliest wins, so the order of the ranges doesn't matter.
fn majority(ranges: &[(i128, i128)]) -> Option<Vec<usize>> {
    let covering = |point: i128| {
        (0..ranges.len())
            .filter(|&at| ranges[at].0 <= point && point <= ranges[at].1)
            .collect::<Vec<_>>()
    };
    let (point, count) = ranges
        .iter()
        .map(|&(low, _)| (low, covering(low).len()))
        .min_by_key(|&(point, count)| (std::cmp::Reverse(count), point))?;
    (count * 2 > ranges.len()).then(|| covering(point))
}

/// One server's answer: its sample, and any leap second it announces.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Answer {
    sample: Sample,
    leap: Option<Leap>,
}

/// A UDP socket connected to `server`, with the reply timeout.
fn connected(server: SocketAddr) -> Result<UdpSocket, ClockError> {
    let local: SocketAddr = match server {
        SocketAddr::V4(_) => (Ipv4Addr::UNSPECIFIED, 0).into(),
        SocketAddr::V6(_) => (Ipv6Addr::UNSPECIFIED, 0).into(),
    };
    let socket = UdpSocket::bind(local)?;
    socket.connect(server)?;
    socket.set_read_timeout(Some(TIMEOUT))?;
    Ok(socket)
}

fn query(server: SocketAddr, time: &impl TimeSource) -> Result<Answer, ClockError> {
    let socket = connected(server)?;
    let sent = time.wall();
    socket.send(&request(sent))?;
    let mut reply = [0; PACKET];
    let length = socket.recv(&mut reply)?;
    let received = time.wall();
    answer(&reply[..length], sent, received)
}

/// Asks an NTS server, running a key exchange first when its association
/// has no cookie left, and gives the association back.
fn query_nts(
    host: &str,
    association: Option<Association>,
    servers: &Servers,
    time: &impl TimeSource,
) -> (Reply, Option<Association>) {
    let tls = servers
        .tls
        .as_ref()
        .expect("NTS servers bring TLS settings");
    let association = match association.filter(|association| !association.cookies.is_empty()) {
        Some(association) => association,
        None => match nts::exchange(host, servers.resolver.as_ref(), tls) {
            Ok(association) => association,
            Err(failed) => return (Reply::unauthenticated(Err(failed)), None),
        },
    };
    let mut association = association;
    let reply = ask_nts(&mut association, servers.resolver.as_ref(), time)
        .unwrap_or_else(|failed| Reply::unauthenticated(Err(failed)));
    (reply, Some(association))
}

/// One NTS exchange. A NAK, the server saying it no longer knows our
/// cookies, empties the association, so the next sync exchanges keys again.
fn ask_nts(
    association: &mut Association,
    resolver: &dyn Resolve,
    time: &impl TimeSource,
) -> Result<Reply, ClockError> {
    let socket = connected(resolver.resolve(&association.server)?)?;
    let sent = time.wall();
    let unique = nts::random();
    let packet = association
        .request(request(sent), unique)
        .ok_or_else(|| ClockError::Nts("no cookie is left".to_owned()))?;
    socket.send(&packet)?;
    let mut reply = [0; REPLY];
    let length = socket.recv(&mut reply)?;
    let received = time.wall();
    let reply = &reply[..length];
    if let Some(header) = association.verify(reply, &unique) {
        return Ok(Reply {
            result: answer(header, sent, received),
            authenticated: true,
        });
    }
    if nak(reply, sent) {
        association.cookies.clear();
        return Err(ClockError::Nts(
            "the server no longer knows our cookies".to_owned(),
        ));
    }
    Err(ClockError::Nts("a reply isn't authenticated".to_owned()))
}

/// Whether `reply` is an NTS NAK to the request sent at `sent`: a
/// Kiss-o'-Death `NTSN`, which comes unauthenticated.
fn nak(reply: &[u8], sent: u64) -> bool {
    reply.len() >= PACKET
        && reply[0] & 0b111 == 4
        && reply[1] == 0
        && &reply[12..16] == b"NTSN"
        && reply[24..32] == to_ntp(sent).to_be_bytes()
}

/// A client request, version 4, with `sent` as its transmit time, which the
/// server echoes back as the origin.
fn request(sent: u64) -> [u8; PACKET] {
    let mut packet = [0; PACKET];
    packet[0] = 0x23;
    packet[40..48].copy_from_slice(&to_ntp(sent).to_be_bytes());
    packet
}

/// What a server's reply gives, from the system clock's readings when the
/// request was sent and the reply received. A reply with stratum 0 is a
/// Kiss-o'-Death, whose code is in the reference id.
fn answer(reply: &[u8], sent: u64, received: u64) -> Result<Answer, ClockError> {
    let reply: &[u8; PACKET] = reply
        .try_into()
        .map_err(|_| ClockError::Reply("not 48 bytes"))?;
    let word =
        |at: usize| u32::from_be_bytes([reply[at], reply[at + 1], reply[at + 2], reply[at + 3]]);
    let stamp = |at: usize| (u64::from(word(at)) << 32) + u64::from(word(at + 4));
    if reply[0] & 0b111 != 4 {
        return Err(ClockError::Reply("not a server's reply"));
    }
    if stamp(24) != to_ntp(sent) {
        return Err(ClockError::Reply("it answers another request"));
    }
    if reply[1] == 0 {
        return Err(ClockError::Kiss(word(12).to_be_bytes()));
    }
    let leap = match reply[0] >> 6 {
        0 => None,
        1 => Some(Leap::Insert),
        2 => Some(Leap::Delete),
        _ => return Err(ClockError::Reply("the server isn't synchronized")),
    };
    if reply[1] > 15 {
        return Err(ClockError::Reply("the server isn't synchronized"));
    }
    let (t1, t4) = (i128::from(sent), i128::from(received));
    let t2 = from_ntp(stamp(32), sent);
    let t3 = from_ntp(stamp(40), sent);
    let offset = ((t2 - t1) + (t3 - t4)) / 2;
    let round_trip = ((t4 - t1) - (t3 - t2)).max(0);
    let bound = round_trip / 2 + short(word(4)) / 2 + short(word(8));
    let sample = Sample {
        offset: i64::try_from(offset).map_err(|_| ClockError::Reply("offset out of range"))?,
        bound: u64::try_from(bound).unwrap_or(u64::MAX),
    };
    Ok(Answer { sample, leap })
}

/// Runs the reply parser on arbitrary bytes, for the fuzzer. The first 16
/// bytes are the send and receive times. When the first of them is odd,
/// the reply's origin is set to match the send time, so the arithmetic
/// past that check is reached too.
#[cfg(fuzzing)]
pub fn fuzz_answer(data: &[u8]) {
    let Some((times, reply)) = data.split_first_chunk::<16>() else {
        return;
    };
    let sent = u64::from_le_bytes(times[..8].try_into().expect("8 bytes"));
    let received = u64::from_le_bytes(times[8..].try_into().expect("8 bytes"));
    let mut reply = reply.to_vec();
    if sent % 2 == 1 && reply.len() >= 32 {
        reply[24..32].copy_from_slice(&to_ntp(sent).to_be_bytes());
    }
    let _ = answer(&reply, sent, received);
}

/// NTP's short format, 16.16 fixed-point seconds, in nanoseconds.
fn short(value: u32) -> i128 {
    (i128::from(value) * NANOS) >> 16
}

/// Unix nanoseconds as an NTP timestamp: 32.32 fixed-point seconds since
/// 1900, wrapping every 2^32 seconds.
fn to_ntp(nanos: u64) -> u64 {
    let nanos = i128::from(nanos);
    let seconds = (nanos / NANOS + UNIX_EPOCH_SECONDS) as u32;
    let fraction = ((nanos % NANOS) << 32) / NANOS;
    (u64::from(seconds) << 32) + fraction as u64
}

/// An NTP timestamp as Unix nanoseconds, in whichever 2^32-second era puts
/// it nearest `near`.
fn from_ntp(stamp: u64, near: u64) -> i128 {
    let seconds = i128::from(stamp >> 32);
    let near = i128::from(near) / NANOS + UNIX_EPOCH_SECONDS;
    let era = (near - seconds + (1 << 31)).div_euclid(1 << 32);
    let seconds = seconds + (era << 32) - UNIX_EPOCH_SECONDS;
    seconds * NANOS + ((i128::from(stamp & 0xffff_ffff) * NANOS) >> 32)
}

/// The range every sample allows, or [`ClockError::Disagree`] when two
/// don't overlap.
fn intersect(samples: &[Sample]) -> Result<Sample, ClockError> {
    let range = |sample: &Sample| {
        let offset = i128::from(sample.offset);
        let bound = i128::from(sample.bound);
        (offset - bound, offset + bound)
    };
    let (low, high) = samples
        .iter()
        .map(range)
        .reduce(|(low, high), (l, h)| (low.max(l), high.min(h)))
        .ok_or(ClockError::NoServers)?;
    if low > high {
        return Err(ClockError::Disagree);
    }
    let middle = (low + high).div_euclid(2);
    Ok(Sample {
        // Both ends came from i64 offsets, so their middle is one too.
        offset: middle as i64,
        bound: u64::try_from(high - middle).unwrap_or(u64::MAX),
    })
}

#[cfg(test)]
mod tests {
    use proptest::{collection::vec, prelude::*};

    use super::*;

    /// 2026-10-08T00:00:00Z.
    const NOW: u64 = 1_791_417_600_000_000_000;

    /// 2100-01-01T00:00:00Z, in NTP's second era.
    const LATER: u64 = 4_102_444_800_000_000_000;

    fn samples() -> impl Strategy<Value = Vec<Sample>> {
        let sample = (
            -1_000_000_000_000i64..1_000_000_000_000,
            0u64..10_000_000_000,
        )
            .prop_map(|(offset, bound)| Sample { offset, bound });
        vec(sample, 1..6)
    }

    proptest! {
        #[test]
        fn the_majority_shares_a_point_and_doesnt_depend_on_order(
            ranges in vec((-1_000i128..1_000, 0i128..500), 1..8),
        ) {
            let ranges: Vec<_> = ranges.into_iter().map(|(at, bound)| (at - bound, at + bound)).collect();
            let mut reversed = ranges.clone();
            reversed.reverse();
            let kept = |ranges: &[(i128, i128)], indices: Option<Vec<usize>>| {
                indices.map(|indices| {
                    let mut kept: Vec<_> = indices.into_iter().map(|at| ranges[at]).collect();
                    kept.sort_unstable();
                    kept
                })
            };
            let forward = kept(&ranges, majority(&ranges));
            prop_assert_eq!(&forward, &kept(&reversed, majority(&reversed)));
            // The most ranges any point is in.
            let most = ranges
                .iter()
                .map(|&(low, _)| ranges.iter().filter(|r| r.0 <= low && low <= r.1).count())
                .max()
                .unwrap();
            match forward {
                Some(kept) => {
                    prop_assert_eq!(kept.len(), most);
                    prop_assert!(most * 2 > ranges.len());
                    let low = kept.iter().map(|r| r.0).max().unwrap();
                    let high = kept.iter().map(|r| r.1).min().unwrap();
                    prop_assert!(low <= high);
                }
                None => prop_assert!(most * 2 <= ranges.len()),
            }
        }

        #[test]
        fn a_timestamp_round_trips_in_either_era(nanos in 0..LATER) {
            let back = from_ntp(to_ntp(nanos), nanos);
            prop_assert!((back - i128::from(nanos)).abs() <= 1);
        }

        #[test]
        fn a_timestamp_reads_back_from_up_to_68_years_away(
            nanos in NOW..LATER,
            away in -2_000_000_000i64..2_000_000_000,
        ) {
            let near = nanos.saturating_add_signed(away * 1_000_000_000);
            let back = from_ntp(to_ntp(nanos), near);
            prop_assert!((back - i128::from(nanos)).abs() <= 1);
        }

        #[test]
        fn the_intersection_is_inside_every_range(samples in samples()) {
            let low = samples.iter().map(|s| range(s).0).max().unwrap();
            let high = samples.iter().map(|s| range(s).1).min().unwrap();
            match intersect(&samples) {
                Ok(met) => {
                    prop_assert!(low <= high);
                    for sample in &samples {
                        let (l, h) = range(sample);
                        prop_assert!((l..=h).contains(&i128::from(met.offset)));
                    }
                    // The met range covers the intersection, rounding up by
                    // at most a nanosecond.
                    let (l, h) = range(&met);
                    prop_assert!(l <= low && high <= h && (h - l) - (high - low) <= 1);
                }
                Err(ClockError::Disagree) => prop_assert!(low > high),
                Err(other) => prop_assert!(false, "{other:?}"),
            }
        }

        #[test]
        fn the_intersection_doesnt_depend_on_order(samples in samples()) {
            let mut reversed = samples.clone();
            reversed.reverse();
            prop_assert_eq!(intersect(&samples).ok(), intersect(&reversed).ok());
        }

        #[test]
        fn any_reply_is_parsed_or_refused_without_panicking(
            bytes in vec(any::<u8>(), 0..64),
            sent: u64,
            received: u64,
            origin_matches: bool,
        ) {
            let mut bytes = bytes;
            if origin_matches && bytes.len() >= 32 {
                bytes[24..32].copy_from_slice(&to_ntp(sent).to_be_bytes());
            }
            let _ = answer(&bytes, sent, received);
        }
    }

    fn reply(origin: u64, receive: u64, transmit: u64) -> [u8; PACKET] {
        let mut packet = [0; PACKET];
        packet[0] = 0x24;
        packet[1] = 2;
        packet[4..8].copy_from_slice(&0x0000_8000u32.to_be_bytes());
        packet[8..12].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        packet[24..32].copy_from_slice(&to_ntp(origin).to_be_bytes());
        packet[32..40].copy_from_slice(&to_ntp(receive).to_be_bytes());
        packet[40..48].copy_from_slice(&to_ntp(transmit).to_be_bytes());
        packet
    }

    #[test]
    fn a_timestamp_round_trips_to_the_nanosecond() {
        for nanos in [NOW, NOW + 999_999_999, 0, 1] {
            let back = from_ntp(to_ntp(nanos), nanos);
            assert!((back - i128::from(nanos)).abs() <= 1, "{nanos}");
        }
    }

    #[test]
    fn a_timestamp_in_the_next_era_reads_after_2036() {
        // 2036-02-07T06:28:16Z, when NTP's seconds wrap to zero.
        let wrap = (1u64 << 32) - 2_208_988_800;
        let after = (wrap + 10) * 1_000_000_000;
        assert_eq!(to_ntp(after) >> 32, 10);
        assert_eq!(from_ntp(to_ntp(after), after), i128::from(after));
        let before = (wrap - 10) * 1_000_000_000;
        assert_eq!(from_ntp(to_ntp(before), after), i128::from(before));
        // 2100-01-01T00:00:00Z, deep in the second era.
        let later = 4_102_444_800 * 1_000_000_000;
        assert_eq!(from_ntp(to_ntp(later), later), i128::from(later));
    }

    #[test]
    fn a_unix_time_is_seconds_since_1900() {
        assert_eq!(to_ntp(0), 2_208_988_800 << 32);
        assert_eq!(to_ntp(500_000_000), (2_208_988_800 << 32) | (1 << 31));
        assert_eq!(from_ntp((2_208_988_800 << 32) | (1 << 31), 0), NANOS / 2);
    }

    #[test]
    fn a_short_is_sixteen_bits_of_seconds() {
        assert_eq!(short(0x0001_0000), NANOS);
        assert_eq!(short(0x0000_8000), NANOS / 2);
    }

    #[test]
    fn a_request_is_a_version_4_client_packet_carrying_its_send_time() {
        let packet = request(NOW);
        assert_eq!(packet[0], 0x23);
        assert_eq!(packet[1..40], [0; 39]);
        assert_eq!(packet[40..48], to_ntp(NOW).to_be_bytes());
    }

    #[test]
    fn a_reply_gives_the_offset_and_the_root_distance() {
        // The server is 5 s ahead; the request takes 10 ms out and 30 ms
        // back, and the server holds it 2 ms.
        let ms = 1_000_000;
        let server = 5_000 * ms;
        let packet = reply(NOW, NOW + server + 10 * ms, NOW + server + 12 * ms);
        let sample = answer(&packet, NOW, NOW + 42 * ms).unwrap().sample;
        // ((10 + 5000) + (5012 - 42)) / 2 = 4990 ms, the round trip is
        // 42 - 2 = 40 ms, and the root delay and dispersion are 0.5 s and 1 s.
        assert!((sample.offset - 4_990 * ms as i64).abs() <= 1);
        assert!(sample.bound.abs_diff(20 * ms + 250 * ms + 1_000 * ms) <= 1);
    }

    #[test]
    fn a_reply_whose_clock_ran_backwards_has_no_negative_round_trip() {
        let packet = reply(NOW, NOW + 50, NOW + 100);
        let sample = answer(&packet, NOW, NOW + 10).unwrap().sample;
        assert_eq!(sample.bound, 250_000_000 + 1_000_000_000);
    }

    #[test]
    fn stratum_0_is_a_kiss_o_death_with_its_code() {
        let mut kiss = reply(NOW, NOW, NOW);
        kiss[0] = 0xe4;
        kiss[1] = 0;
        kiss[12..16].copy_from_slice(b"RATE");
        assert!(matches!(answer(&kiss, NOW, NOW), Err(ClockError::Kiss(code)) if &code == b"RATE"));
        assert_eq!(
            ClockError::Kiss(*b"DENY").to_string(),
            "the NTP server sent Kiss-o'-Death DENY"
        );
    }

    #[test]
    fn a_reply_carries_the_leap_second_it_announces() {
        let leap = |indicator: u8| {
            let mut packet = reply(NOW, NOW, NOW);
            packet[0] = indicator << 6 | 0x24;
            answer(&packet, NOW, NOW).unwrap().leap
        };
        assert_eq!(leap(0), None);
        assert_eq!(leap(1), Some(Leap::Insert));
        assert_eq!(leap(2), Some(Leap::Delete));
    }

    #[test]
    fn the_majority_is_more_than_half() {
        assert_eq!(majority(&[(0, 10), (5, 15), (20, 30)]), Some(vec![0, 1]));
        assert_eq!(majority(&[(0, 10), (20, 30)]), None);
        assert_eq!(majority(&[(0, 10), (10, 20), (30, 40), (31, 41)]), None);
        assert_eq!(
            majority(&[(0, 10), (10, 20), (10, 15), (30, 40)]),
            Some(vec![0, 1, 2])
        );
        assert_eq!(majority(&[(3, 3)]), Some(vec![0]));
        assert_eq!(majority(&[]), None);
        // Two points shared by three of four: the earlier wins.
        let ranges = [(0, 10), (5, 20), (5, 20), (15, 30)];
        assert_eq!(majority(&ranges), Some(vec![0, 1, 2]));
    }

    #[test]
    fn a_rate_kiss_doubles_the_poll_and_holds_the_server() {
        let mut schedule = Schedule::default();
        assert!(schedule.askable(0));
        schedule.kissed(*b"RATE", 1_000, false);
        assert_eq!(schedule.poll, 2 * MIN_POLL);
        assert!(!schedule.askable(1_000 + 2 * MIN_POLL - 1));
        assert!(schedule.askable(1_000 + 2 * MIN_POLL));
        for _ in 0..20 {
            schedule.kissed(*b"RATE", 0, false);
        }
        // An unauthenticated RATE never holds a server longer than 1024 s.
        assert_eq!(schedule.poll, MAX_HOLD);
        assert!(schedule.askable(MAX_HOLD));
    }

    #[test]
    fn only_a_raised_poll_holds_a_server_after_it_is_asked() {
        let mut schedule = Schedule::default();
        schedule.asked(1_000);
        assert!(schedule.askable(1_000));
        schedule.kissed(*b"RATE", 0, false);
        schedule.asked(1_000);
        assert!(!schedule.askable(1_000 + 2 * MIN_POLL - 1));
        assert!(schedule.askable(1_000 + 2 * MIN_POLL));
    }

    #[test]
    fn a_deny_or_rstr_kiss_stops_the_server_and_others_change_nothing() {
        for code in [*b"DENY", *b"RSTR"] {
            let mut schedule = Schedule::default();
            schedule.kissed(code, 1_000, false);
            assert!(schedule.denied);
            assert!(!schedule.askable(1_000 + MAX_HOLD - 1));
            assert!(schedule.askable(1_000 + MAX_HOLD));
        }
        let mut schedule = Schedule::default();
        schedule.kissed(*b"INIT", 5, false);
        assert!(schedule.askable(0));
        assert_eq!(schedule.poll, MIN_POLL);
    }

    #[test]
    fn a_bad_reply_is_refused() {
        let good = reply(NOW, NOW, NOW);
        let refused = |packet: &[u8], why: &str| match answer(packet, NOW, NOW) {
            Err(ClockError::Reply(reason)) => assert_eq!(reason, why),
            other => panic!("{other:?}"),
        };
        refused(&good[..47], "not 48 bytes");
        let mut client = good;
        client[0] = 0x23;
        refused(&client, "not a server's reply");
        let mut alarm = good;
        alarm[0] = 0xe4;
        refused(&alarm, "the server isn't synchronized");
        let mut unsynced = good;
        unsynced[1] = 16;
        refused(&unsynced, "the server isn't synchronized");
        let mut stratum_15 = good;
        stratum_15[1] = 15;
        assert!(answer(&stratum_15, NOW, NOW).is_ok());
        refused(&reply(NOW + 1_000, NOW, NOW), "it answers another request");
        let mut kiss = reply(NOW + 1_000, NOW, NOW);
        kiss[1] = 0;
        refused(&kiss, "it answers another request");
        // Answered at 1970 and received at the end of u64 time.
        match answer(&reply(NOW, 0, 0), NOW, u64::MAX) {
            Err(ClockError::Reply(reason)) => assert_eq!(reason, "offset out of range"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn samples_meet_in_their_intersection() {
        let sample = |offset, bound| Sample { offset, bound };
        assert_eq!(
            intersect(&[sample(100, 50), sample(130, 40)]).unwrap(),
            sample(120, 30)
        );
        assert_eq!(intersect(&[sample(-7, 2)]).unwrap(), sample(-7, 2));
        assert_eq!(
            intersect(&[sample(0, 5), sample(10, 5)]).unwrap(),
            sample(5, 0)
        );
        assert_eq!(
            intersect(&[sample(0, 1), sample(3, 1), sample(1, 1)]).ok(),
            None
        );
        assert!(matches!(intersect(&[]), Err(ClockError::NoServers)));
        assert!(matches!(
            intersect(&[sample(0, 4), sample(9, 4)]),
            Err(ClockError::Disagree)
        ));
    }

    #[test]
    fn an_odd_width_rounds_the_bound_up() {
        let sample = |offset, bound| Sample { offset, bound };
        // [-3, 0]: the middle is -2 and the bound 2, which covers -4..=0.
        assert_eq!(
            intersect(&[sample(-1, 2), sample(-2, 2)]).unwrap(),
            sample(-2, 2)
        );
    }
}
