//! An SNTP client, for when the platform reports no bound of its own.
//!
//! Each server is asked once per sync. A reply gives the system clock's
//! offset from the server and a bound: half the round trip, plus half the
//! server's root delay, plus its root dispersion, which is RFC 5905's root
//! distance. The clock takes the intersection of every reply's range, so a
//! server that is wrong and sure of itself fails the sync instead of moving
//! the clock.

use std::{
    future::Future,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs, UdpSocket},
    time::Duration,
};

use sungma::clock::{Clock, Reading};

use crate::{
    ClockError,
    wall::{self, Sample, Sampled},
};

/// Seconds from 1900, NTP's epoch, to 1970, the Unix epoch.
const UNIX_EPOCH_SECONDS: i128 = 2_208_988_800;
const NANOS: i128 = 1_000_000_000;
const PACKET: usize = 48;
const TIMEOUT: Duration = Duration::from_secs(1);

/// The system clock, corrected by the servers' offset, give or take their
/// bound. Syncing blocks on the network, so an async caller runs it on a
/// blocking thread.
#[derive(Debug)]
pub struct NtpClock {
    servers: Vec<SocketAddr>,
    sampled: Sampled,
}

impl NtpClock {
    /// Resolves `servers` and takes a first sample. A server that doesn't
    /// answer is skipped, and every one that does must agree.
    pub fn sync<A: ToSocketAddrs>(servers: &[A]) -> Result<Self, ClockError> {
        let mut addresses = Vec::new();
        for server in servers {
            addresses.extend(server.to_socket_addrs()?);
        }
        let sampled = Sampled::new(sample(&addresses)?);
        Ok(Self {
            servers: addresses,
            sampled,
        })
    }

    /// Takes a fresh sample. On an error the old sample stays, and its
    /// bound keeps growing.
    pub fn resync(&self) -> Result<(), ClockError> {
        self.sampled.replace(sample(&self.servers)?);
        Ok(())
    }
}

impl Clock for NtpClock {
    fn now(&self) -> Reading {
        self.sampled.now()
    }

    fn wait(&self, stamped: Reading) -> impl Future<Output = ()> + Send {
        wall::wait(self, stamped)
    }
}

fn sample(servers: &[SocketAddr]) -> Result<Sample, ClockError> {
    let mut samples = Vec::new();
    let mut error = ClockError::NoServers;
    for &server in servers {
        match query(server) {
            Ok(sample) => samples.push(sample),
            Err(failed) => error = failed,
        }
    }
    if samples.is_empty() {
        return Err(error);
    }
    intersect(&samples)
}

fn query(server: SocketAddr) -> Result<Sample, ClockError> {
    let local: SocketAddr = match server {
        SocketAddr::V4(_) => (Ipv4Addr::UNSPECIFIED, 0).into(),
        SocketAddr::V6(_) => (Ipv6Addr::UNSPECIFIED, 0).into(),
    };
    let socket = UdpSocket::bind(local)?;
    socket.connect(server)?;
    socket.set_read_timeout(Some(TIMEOUT))?;
    let sent = wall::wall_nanos();
    socket.send(&request(sent))?;
    let mut reply = [0; PACKET];
    let length = socket.recv(&mut reply)?;
    let received = wall::wall_nanos();
    answer(&reply[..length], sent, received)
}

/// A client request, version 4, with `sent` as its transmit time, which the
/// server echoes back as the origin.
fn request(sent: u64) -> [u8; PACKET] {
    let mut packet = [0; PACKET];
    packet[0] = 0x23;
    packet[40..48].copy_from_slice(&to_ntp(sent).to_be_bytes());
    packet
}

/// The sample a server's reply gives, from the system clock's readings when
/// the request was sent and the reply received.
fn answer(reply: &[u8], sent: u64, received: u64) -> Result<Sample, ClockError> {
    let reply: &[u8; PACKET] = reply
        .try_into()
        .map_err(|_| ClockError::Reply("not 48 bytes"))?;
    let word =
        |at: usize| u32::from_be_bytes([reply[at], reply[at + 1], reply[at + 2], reply[at + 3]]);
    let stamp = |at: usize| (u64::from(word(at)) << 32) + u64::from(word(at + 4));
    if reply[0] & 0b111 != 4 {
        return Err(ClockError::Reply("not a server's reply"));
    }
    if reply[0] >> 6 == 3 || !(1..=15).contains(&reply[1]) {
        return Err(ClockError::Reply("the server isn't synchronized"));
    }
    if stamp(24) != to_ntp(sent) {
        return Err(ClockError::Reply("it answers another request"));
    }
    let (t1, t4) = (i128::from(sent), i128::from(received));
    let t2 = from_ntp(stamp(32), sent);
    let t3 = from_ntp(stamp(40), sent);
    let offset = ((t2 - t1) + (t3 - t4)) / 2;
    let round_trip = ((t4 - t1) - (t3 - t2)).max(0);
    let bound = round_trip / 2 + short(word(4)) / 2 + short(word(8));
    Ok(Sample {
        offset: i64::try_from(offset).map_err(|_| ClockError::Reply("offset out of range"))?,
        bound: u64::try_from(bound).unwrap_or(u64::MAX),
    })
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
    use super::*;

    /// 2026-10-08T00:00:00Z.
    const NOW: u64 = 1_791_417_600_000_000_000;

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
        let sample = answer(&packet, NOW, NOW + 42 * ms).unwrap();
        // ((10 + 5000) + (5012 - 42)) / 2 = 4990 ms, the round trip is
        // 42 - 2 = 40 ms, and the root delay and dispersion are 0.5 s and 1 s.
        assert!((sample.offset - 4_990 * ms as i64).abs() <= 1);
        assert!(sample.bound.abs_diff(20 * ms + 250 * ms + 1_000 * ms) <= 1);
    }

    #[test]
    fn a_reply_whose_clock_ran_backwards_has_no_negative_round_trip() {
        let packet = reply(NOW, NOW + 50, NOW + 100);
        let sample = answer(&packet, NOW, NOW + 10).unwrap();
        assert_eq!(sample.bound, 250_000_000 + 1_000_000_000);
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
        for stratum in [0, 16] {
            let mut unsynced = good;
            unsynced[1] = stratum;
            refused(&unsynced, "the server isn't synchronized");
        }
        let mut stratum_15 = good;
        stratum_15[1] = 15;
        assert!(answer(&stratum_15, NOW, NOW).is_ok());
        refused(&reply(NOW + 1_000, NOW, NOW), "it answers another request");
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
