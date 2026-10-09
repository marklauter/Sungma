//! Each clock against its contract, with NTP servers simulated on the
//! loopback interface.

use std::{
    net::{SocketAddr, UdpSocket},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use sungma::clock::{Clock, Reading, Revision};
use sungma_clock::{ClockError, DevClock, ManualClock, NtpClock, SystemClock};
use tokio::time::timeout;

const SECOND: u64 = 1_000_000_000;

fn wall() -> u64 {
    let since = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    u64::try_from(since.as_nanos()).unwrap()
}

fn at(settled: u64, revision: u64) -> Reading {
    Reading {
        settled: Revision(settled),
        revision: Revision(revision),
    }
}

/// An NTP server that answers one request per entry in `ahead`, each time
/// that many seconds ahead of the client, with a root dispersion of 10 ms.
fn server(address: &str, ahead: Vec<u32>) -> SocketAddr {
    let socket = UdpSocket::bind(address).unwrap();
    let address = socket.local_addr().unwrap();
    thread::spawn(move || {
        for seconds in ahead {
            let mut request = [0; 48];
            let (_, client) = socket.recv_from(&mut request).unwrap();
            let sent = u64::from_be_bytes(request[40..48].try_into().unwrap());
            let now = sent.wrapping_add(u64::from(seconds) << 32).to_be_bytes();
            let mut reply = [0; 48];
            reply[0] = 0x24;
            reply[1] = 2;
            // 10 ms of root dispersion, in 16.16 fixed-point seconds.
            reply[8..12].copy_from_slice(&655u32.to_be_bytes());
            reply[24..32].copy_from_slice(&request[40..48]);
            reply[32..40].copy_from_slice(&now);
            reply[40..48].copy_from_slice(&now);
            socket.send_to(&reply, client).unwrap();
        }
    });
    address
}

/// A server that never answers.
fn silent() -> (UdpSocket, SocketAddr) {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let address = socket.local_addr().unwrap();
    (socket, address)
}

/// Whether `clock` reads true time as the system clock plus `ahead`
/// seconds, within a bound of 10 ms plus the loopback round trip.
fn reads_ahead(clock: &impl Clock, ahead: u64) {
    let before = wall() + ahead * SECOND;
    let now = clock.now();
    let after = wall() + ahead * SECOND;
    assert!(
        now.settled.0 <= after && before <= now.revision.0,
        "{now:?}"
    );
    let width = now.revision.0 - now.settled.0;
    assert!((20_000_000..100_000_000).contains(&width), "{width}");
}

#[test]
fn a_dev_clock_counts_one_past_the_last_reading() {
    let clock = DevClock::default();
    assert_eq!(clock.now(), at(1, 1));
    assert_eq!(clock.now(), at(2, 2));
    let restarted = DevClock::starting_after(Revision(41));
    assert_eq!(restarted.now(), at(42, 42));
}

#[tokio::test]
async fn a_dev_clock_wait_moves_the_counter_past_the_revision() {
    let clock = DevClock::default();
    clock.wait(at(9, 9)).await;
    assert_eq!(clock.now(), at(10, 10));
    clock.wait(at(3, 3)).await;
    assert_eq!(clock.now(), at(11, 11));
}

#[tokio::test]
async fn a_manual_clock_holds_a_wait_until_settled_passes() {
    let clock = ManualClock::new(at(5, 9));
    assert_eq!(clock.now(), at(5, 9));
    let wait = clock.wait(at(5, 9));
    tokio::pin!(wait);
    assert!(timeout(Duration::from_millis(20), &mut wait).await.is_err());
    clock.set(at(9, 13));
    assert_eq!(clock.now(), at(9, 13));
    assert!(timeout(Duration::from_millis(20), &mut wait).await.is_err());
    clock.set(at(10, 14));
    timeout(Duration::from_secs(1), wait).await.unwrap();
}

#[test]
fn an_ntp_clock_reads_the_servers_time() {
    let clock = NtpClock::sync(&[server("127.0.0.1:0", vec![3])]).unwrap();
    reads_ahead(&clock, 3);
}

#[test]
fn an_ntp_clock_reaches_a_server_over_ipv6() {
    if UdpSocket::bind("[::1]:0").is_ok() {
        let clock = NtpClock::sync(&[server("[::1]:0", vec![0])]).unwrap();
        reads_ahead(&clock, 0);
    }
}

#[test]
fn servers_that_disagree_fail_the_sync() {
    let servers = [
        server("127.0.0.1:0", vec![0]),
        server("127.0.0.1:0", vec![100]),
    ];
    assert!(matches!(
        NtpClock::sync(&servers),
        Err(ClockError::Disagree)
    ));
}

#[test]
fn a_server_that_doesnt_answer_is_skipped() {
    let (_socket, quiet) = silent();
    let clock = NtpClock::sync(&[quiet, server("127.0.0.1:0", vec![2])]).unwrap();
    reads_ahead(&clock, 2);
}

#[test]
fn a_sync_with_no_answer_fails() {
    let (_socket, quiet) = silent();
    assert!(matches!(NtpClock::sync(&[quiet]), Err(ClockError::Io(_))));
    let none: [&str; 0] = [];
    assert!(matches!(NtpClock::sync(&none), Err(ClockError::NoServers)));
}

#[test]
fn a_resync_takes_the_new_sample_and_a_failed_one_keeps_the_old() {
    let clock = NtpClock::sync(&[server("127.0.0.1:0", vec![0, 50])]).unwrap();
    reads_ahead(&clock, 0);
    clock.resync().unwrap();
    reads_ahead(&clock, 50);
    assert!(matches!(clock.resync(), Err(ClockError::Io(_))));
    reads_ahead(&clock, 50);
}

#[tokio::test]
async fn an_ntp_clock_waits_until_settled_passes() {
    let clock = NtpClock::sync(&[server("127.0.0.1:0", vec![0])]).unwrap();
    let stamped = clock.now();
    clock.wait(stamped).await;
    assert!(clock.now().settled > stamped.revision);
}

#[tokio::test]
async fn the_system_clock_reads_true_time_and_waits_it_out() {
    // A server in step with the system clock, for a platform without a
    // bound of its own.
    let clock = SystemClock::detect(&[server("127.0.0.1:0", vec![0])]).unwrap();
    let before = wall();
    let now = clock.now();
    assert!(
        now.settled.0 <= wall() && before <= now.revision.0,
        "{now:?}"
    );
    clock.wait(now).await;
    assert!(clock.now().settled > now.revision);
}

#[test]
fn a_system_clock_over_ntp_refreshes_with_a_new_sample() {
    let ntp = NtpClock::sync(&[server("127.0.0.1:0", vec![0, 50])]).unwrap();
    let clock = SystemClock::Ntp(ntp);
    reads_ahead(&clock, 0);
    clock.refresh().unwrap();
    reads_ahead(&clock, 50);
}

#[test]
fn a_system_clock_refresh_keeps_it_reading() {
    let clock = SystemClock::detect(&[server("127.0.0.1:0", vec![0, 0])]).unwrap();
    clock.refresh().unwrap();
    let before = wall();
    let now = clock.now();
    assert!(
        now.settled.0 <= wall() && before <= now.revision.0,
        "{now:?}"
    );
}
