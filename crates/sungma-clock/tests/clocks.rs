//! Each clock against its contract, with NTP servers simulated on the
//! loopback interface.

use std::{
    net::{SocketAddr, UdpSocket},
    sync::Arc,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use sungma::clock::{Clock, ClockFault, Reading, Revision};
use sungma_clock::{
    ClockError, DevClock, ManualClock, ManualTime, NtpClock, SystemClock, TimeSource,
};
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
    let now = clock.now().unwrap();
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
    assert_eq!(clock.now(), Ok(at(1, 1)));
    assert_eq!(clock.now(), Ok(at(2, 2)));
    let restarted = DevClock::starting_after(Revision(41));
    assert_eq!(restarted.now(), Ok(at(42, 42)));
}

#[tokio::test]
async fn a_dev_clock_wait_moves_the_counter_past_the_revision() {
    let clock = DevClock::default();
    clock.wait(at(9, 9)).await.unwrap();
    assert_eq!(clock.now(), Ok(at(10, 10)));
    clock.wait(at(3, 3)).await.unwrap();
    assert_eq!(clock.now(), Ok(at(11, 11)));
}

#[tokio::test]
async fn a_manual_clock_holds_a_wait_until_settled_passes() {
    let clock = ManualClock::new(at(5, 9));
    assert_eq!(clock.now(), Ok(at(5, 9)));
    let wait = clock.wait(at(5, 9));
    tokio::pin!(wait);
    assert!(timeout(Duration::from_millis(20), &mut wait).await.is_err());
    clock.set(at(9, 13));
    assert_eq!(clock.now(), Ok(at(9, 13)));
    assert!(timeout(Duration::from_millis(20), &mut wait).await.is_err());
    clock.set(at(10, 14));
    timeout(Duration::from_secs(1), wait)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn a_manual_fault_ends_a_wait_and_every_reading() {
    let clock = ManualClock::new(at(5, 9));
    let wait = clock.wait(at(5, 9));
    tokio::pin!(wait);
    assert!(timeout(Duration::from_millis(20), &mut wait).await.is_err());
    clock.fault(ClockFault::Drifted);
    let ended = timeout(Duration::from_secs(1), wait).await.unwrap();
    assert_eq!(ended, Err(ClockFault::Drifted));
    assert_eq!(clock.now(), Err(ClockFault::Drifted));
    clock.set(at(5, 9));
    assert_eq!(clock.now(), Ok(at(5, 9)));
}

#[test]
fn a_revision_stamped_past_the_clocks_reading_is_a_fault() {
    let clock = ManualClock::new(at(5, 9));
    assert_eq!(clock.observe(Revision(9)), Ok(()));
    assert_eq!(
        clock.observe(Revision(10)),
        Err(ClockFault::Behind {
            seen: Revision(10),
            revision: Revision(9)
        })
    );
    clock.fault(ClockFault::Unsynchronized);
    assert_eq!(clock.observe(Revision(1)), Err(ClockFault::Unsynchronized));
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
fn a_resync_adds_to_the_window_and_a_failed_one_keeps_it() {
    let clock = NtpClock::sync(&[server("127.0.0.1:0", vec![3, 3])]).unwrap();
    reads_ahead(&clock, 3);
    clock.resync().unwrap();
    reads_ahead(&clock, 3);
    assert!(matches!(clock.resync(), Err(ClockError::Io(_))));
    reads_ahead(&clock, 3);
}

#[test]
fn a_resync_that_disagrees_is_a_drift_and_starts_the_window_over() {
    let clock = NtpClock::sync(&[server("127.0.0.1:0", vec![0, 50])]).unwrap();
    reads_ahead(&clock, 0);
    assert!(matches!(
        clock.resync(),
        Err(ClockError::Fault(ClockFault::Drifted))
    ));
    reads_ahead(&clock, 50);
}

#[tokio::test]
async fn an_ntp_clock_waits_until_settled_passes() {
    let clock = NtpClock::sync(&[server("127.0.0.1:0", vec![0])]).unwrap();
    let stamped = clock.now().unwrap();
    timeout(Duration::from_secs(5), clock.wait(stamped))
        .await
        .unwrap()
        .unwrap();
    assert!(clock.now().unwrap().settled > stamped.revision);
}

#[tokio::test]
async fn the_system_clock_reads_true_time_and_waits_it_out() {
    // A server in step with the system clock, for a platform without a
    // bound of its own.
    let clock = SystemClock::detect(&[server("127.0.0.1:0", vec![0])]).unwrap();
    let before = wall();
    let now = clock.now().unwrap();
    assert!(
        now.settled.0 <= wall() && before <= now.revision.0,
        "{now:?}"
    );
    timeout(Duration::from_secs(5), clock.wait(now))
        .await
        .unwrap()
        .unwrap();
    assert!(clock.now().unwrap().settled > now.revision);
}

#[test]
fn a_system_clock_over_ntp_refreshes_with_a_new_sample() {
    let ntp = NtpClock::sync(&[server("127.0.0.1:0", vec![0, 50])]).unwrap();
    let clock = SystemClock::Ntp(ntp);
    reads_ahead(&clock, 0);
    assert!(matches!(
        clock.refresh(),
        Err(ClockError::Fault(ClockFault::Drifted))
    ));
    reads_ahead(&clock, 50);
}

#[test]
fn a_system_clock_refresh_keeps_it_reading() {
    let clock = SystemClock::detect(&[server("127.0.0.1:0", vec![0, 0])]).unwrap();
    clock.refresh().unwrap();
    let before = wall();
    let now = clock.now().unwrap();
    assert!(
        now.settled.0 <= wall() && before <= now.revision.0,
        "{now:?}"
    );
}

#[test]
fn an_ntp_clock_reads_its_time_source() {
    let start = wall();
    let time = Arc::new(ManualTime::new(start));
    let clock = NtpClock::sync_with(&[server("127.0.0.1:0", vec![3])], time.clone()).unwrap();
    let reading = |clock: &NtpClock<Arc<ManualTime>>| {
        let now = clock.now().unwrap();
        (now.settled.0 + now.revision.0) / 2
    };
    // NTP's fixed-point timestamps cost up to a nanosecond.
    assert!(reading(&clock).abs_diff(start + 3 * SECOND) <= 1);
    time.advance(Duration::from_secs(1));
    assert!(reading(&clock).abs_diff(start + 4 * SECOND) <= 1);
}

#[test]
fn a_small_step_either_way_widens_the_reading_by_its_size() {
    for by in [5_000_000, -5_000_000] {
        let (time, clock) = driven(1);
        let before = clock.now().unwrap();
        time.step(by);
        let after = clock.now().unwrap();
        // True time follows the monotonic clock, so the reading stays put
        // and widens by 5 ms on each side.
        assert_eq!(after.settled.0, before.settled.0 - 5_000_000);
        assert_eq!(after.revision.0, before.revision.0 + 5_000_000);
    }
}

/// An NTP clock on clocks a test drives, synced to a server in step with
/// true time that answers `answers` times.
fn driven(answers: usize) -> (Arc<ManualTime>, NtpClock<Arc<ManualTime>>) {
    let time = Arc::new(ManualTime::new(wall()));
    let server = server("127.0.0.1:0", vec![0; answers]);
    let clock = NtpClock::sync_with(&[server], time.clone()).unwrap();
    (time, clock)
}

#[test]
fn a_step_either_way_faults_the_clock_until_a_resync() {
    for by in [20_000_000, -20_000_000] {
        let (time, clock) = driven(2);
        time.step(by);
        assert_eq!(clock.now(), Err(ClockFault::Jumped { by }));
        clock.resync().unwrap();
        assert!(clock.now().is_ok());
    }
}

#[test]
fn a_suspend_the_monotonic_clock_misses_is_a_jump() {
    let (time, clock) = driven(2);
    // The system clock catches up after a minute asleep; a monotonic clock
    // that stopped during the suspend didn't count it.
    time.step(60_000_000_000);
    assert_eq!(clock.now(), Err(ClockFault::Jumped { by: 60_000_000_000 }));
    clock.resync().unwrap();
    assert!(clock.now().is_ok());
}

#[test]
fn a_suspend_the_monotonic_clock_counts_widens_the_bound() {
    let (time, clock) = driven(1);
    let before = clock.now().unwrap();
    time.advance(Duration::from_secs(60));
    let after = clock.now().unwrap();
    let width = |reading: Reading| reading.revision.0 - reading.settled.0;
    // A minute of drift at 500 ppm is 30 ms each side.
    assert_eq!(width(after) - width(before), 60_000_000);
    // The server is in step with the system clock, which is true time.
    assert!(after.settled.0 <= time.wall() && time.wall() <= after.revision.0);
}

#[test]
fn a_failed_resync_leaves_the_bound_growing_from_each_sample() {
    let (time, clock) = driven(1);
    let width = |reading: Reading| reading.revision.0 - reading.settled.0;
    let before = width(clock.now().unwrap());
    time.advance(Duration::from_secs(10));
    assert!(matches!(clock.resync(), Err(ClockError::Io(_))));
    // Ten seconds of drift since the sample: 5 ms each side.
    assert_eq!(width(clock.now().unwrap()) - before, 10_000_000);
}

#[tokio::test]
async fn a_long_outage_refuses_writes_instead_of_hanging_them() {
    let (time, clock) = driven(1);
    let stamped = clock.now().unwrap();
    // 35 minutes without a sample: drift alone passes a one-second bound.
    time.advance(Duration::from_secs(35 * 60));
    assert_eq!(clock.now(), Err(ClockFault::Unsynchronized));
    let wait = timeout(Duration::from_secs(1), clock.wait(stamped)).await;
    assert_eq!(wait.unwrap(), Err(ClockFault::Unsynchronized));
    assert!(matches!(clock.resync(), Err(ClockError::Io(_))));
    assert_eq!(clock.now(), Err(ClockFault::Unsynchronized));
}
