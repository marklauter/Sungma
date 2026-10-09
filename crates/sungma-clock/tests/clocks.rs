//! Each clock against its contract, with NTP servers simulated on the
//! loopback interface.

use std::{
    net::{SocketAddr, UdpSocket},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use sungma::clock::{Clock, ClockFault, Reading, Revision};
use sungma_clock::{
    ClockError, DevClock, Leap, ManualClock, ManualTime, NtpClock, Refresh, Servers, SystemClock,
    TimeSource,
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

/// What a counting server answers, one request at a time.
#[derive(Clone, Copy)]
enum Answer {
    /// That many seconds ahead of the client.
    Ahead(i32),
    /// Kiss-o'-Death with this code.
    Kiss(&'static [u8; 4]),
    /// In step with the client, announcing this leap indicator.
    Leap(u8),
}

/// A server that gives `answers` in order and then stays silent, counting
/// every request it receives.
fn counting(answers: Vec<Answer>) -> (SocketAddr, Arc<AtomicUsize>) {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let address = socket.local_addr().unwrap();
    let asked = Arc::new(AtomicUsize::new(0));
    let count = asked.clone();
    thread::spawn(move || {
        let mut answers = answers.into_iter();
        loop {
            let mut request = [0; 48];
            let (_, client) = socket.recv_from(&mut request).unwrap();
            count.fetch_add(1, Ordering::SeqCst);
            let Some(answer) = answers.next() else {
                continue;
            };
            let sent = u64::from_be_bytes(request[40..48].try_into().unwrap());
            let mut reply = [0; 48];
            reply[0] = 0x24;
            reply[1] = 2;
            reply[8..12].copy_from_slice(&655u32.to_be_bytes());
            reply[24..32].copy_from_slice(&request[40..48]);
            let mut now = sent;
            match answer {
                Answer::Ahead(seconds) => {
                    now = sent.wrapping_add_signed(i64::from(seconds) << 32);
                }
                Answer::Kiss(code) => {
                    reply[0] = 0xe4;
                    reply[1] = 0;
                    reply[12..16].copy_from_slice(code);
                }
                Answer::Leap(indicator) => reply[0] |= indicator << 6,
            }
            reply[32..40].copy_from_slice(&now.to_be_bytes());
            reply[40..48].copy_from_slice(&now.to_be_bytes());
            socket.send_to(&reply, client).unwrap();
        }
    });
    (address, asked)
}

fn asked(count: &AtomicUsize) -> usize {
    count.load(Ordering::SeqCst)
}

/// Three servers like [`server`], the fewest a sync takes.
fn trio(address: &str, ahead: Vec<u32>) -> Vec<SocketAddr> {
    (0..3).map(|_| server(address, ahead.clone())).collect()
}

/// Three counting servers giving the same answers.
fn counting_trio(answers: Vec<Answer>) -> Vec<SocketAddr> {
    (0..3).map(|_| counting(answers.clone()).0).collect()
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
    let clock = NtpClock::sync(Servers::new(trio("127.0.0.1:0", vec![3]))).unwrap();
    reads_ahead(&clock, 3);
}

#[test]
fn an_ntp_clock_reaches_a_server_over_ipv6() {
    if UdpSocket::bind("[::1]:0").is_ok() {
        let clock = NtpClock::sync(Servers::new(trio("[::1]:0", vec![0]))).unwrap();
        reads_ahead(&clock, 0);
    }
}

#[test]
fn servers_that_disagree_fail_the_sync() {
    let servers = [
        server("127.0.0.1:0", vec![0]),
        server("127.0.0.1:0", vec![100]),
        server("127.0.0.1:0", vec![200]),
    ];
    assert!(matches!(
        NtpClock::sync(Servers::new(servers)),
        Err(ClockError::Disagree)
    ));
}

#[test]
fn a_server_that_doesnt_answer_is_skipped() {
    let (_socket, quiet) = silent();
    let clock = NtpClock::sync(Servers::new(
        [vec![quiet], trio("127.0.0.1:0", vec![2])].concat(),
    ))
    .unwrap();
    reads_ahead(&clock, 2);
}

#[test]
fn a_sync_with_no_answer_fails() {
    let (_socket, quiet) = silent();
    assert!(matches!(
        NtpClock::sync(Servers::new([quiet])),
        Err(ClockError::Io(_))
    ));
    let none: [&str; 0] = [];
    assert!(matches!(
        NtpClock::sync(Servers::new(none)),
        Err(ClockError::NoServers)
    ));
}

#[test]
fn a_resync_adds_to_the_window_and_a_failed_one_keeps_it() {
    let clock = NtpClock::sync(Servers::new(trio("127.0.0.1:0", vec![3, 3]))).unwrap();
    reads_ahead(&clock, 3);
    clock.resync().unwrap();
    reads_ahead(&clock, 3);
    assert!(matches!(clock.resync(), Err(ClockError::Io(_))));
    reads_ahead(&clock, 3);
}

#[test]
fn a_resync_that_disagrees_is_a_drift_and_starts_the_window_over() {
    let clock = NtpClock::sync(Servers::new(trio("127.0.0.1:0", vec![0, 50]))).unwrap();
    reads_ahead(&clock, 0);
    assert!(matches!(
        clock.resync(),
        Err(ClockError::Fault(ClockFault::Drifted))
    ));
    reads_ahead(&clock, 50);
}

#[tokio::test]
async fn an_ntp_clock_waits_until_settled_passes() {
    let clock = NtpClock::sync(Servers::new(trio("127.0.0.1:0", vec![0]))).unwrap();
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
    let clock = SystemClock::detect(Servers::new(trio("127.0.0.1:0", vec![0]))).unwrap();
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
    let ntp = NtpClock::sync(Servers::new(trio("127.0.0.1:0", vec![0, 50]))).unwrap();
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
    let clock = SystemClock::detect(Servers::new(trio("127.0.0.1:0", vec![0, 0]))).unwrap();
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
    let clock =
        NtpClock::sync_with(Servers::new(trio("127.0.0.1:0", vec![3])), time.clone()).unwrap();
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
    let servers = trio("127.0.0.1:0", vec![0; answers]);
    let clock = NtpClock::sync_with(Servers::new(servers), time.clone()).unwrap();
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

#[test]
fn a_falseticker_is_outvoted_and_named() {
    let honest = (0..3).map(|_| counting(vec![Answer::Ahead(0)]).0);
    let (wrong, _) = counting(vec![Answer::Ahead(100)]);
    let servers: Vec<_> = honest.chain([wrong]).collect();
    let clock = NtpClock::sync(Servers::new(servers)).unwrap();
    reads_ahead(&clock, 0);
    assert_eq!(clock.last_sync().falsetickers, vec![wrong.to_string()]);
}

#[test]
fn two_wrong_servers_of_four_fail_the_sync() {
    let servers: Vec<_> = [0, 0, 100, -100]
        .into_iter()
        .map(|ahead| counting(vec![Answer::Ahead(ahead)]).0)
        .collect();
    assert!(matches!(
        NtpClock::sync(Servers::new(servers)),
        Err(ClockError::Disagree)
    ));
}

#[test]
fn a_sync_asks_every_server_at_once() {
    let quiet: Vec<_> = (0..3).map(|_| silent()).collect();
    let good = counting_trio(vec![Answer::Ahead(0)]);
    let servers = quiet.iter().map(|(_, address)| *address).chain(good);
    let started = Instant::now();
    let clock = NtpClock::sync(Servers::new(servers)).unwrap();
    // One timeout for all three silent servers, not one each.
    assert!(started.elapsed() < Duration::from_millis(1_500));
    assert_eq!(clock.last_sync().unanswered.len(), 3);
}

#[test]
fn a_rate_kiss_holds_the_server_off_for_twice_the_poll() {
    let time = Arc::new(ManualTime::new(wall()));
    let (kisser, kissed) = counting(vec![
        Answer::Kiss(b"RATE"),
        Answer::Ahead(0),
        Answer::Ahead(0),
    ]);
    let honest = counting_trio(vec![Answer::Ahead(0); 6]);
    let servers: Vec<_> = [kisser].into_iter().chain(honest).collect();
    let clock = NtpClock::sync_with(Servers::new(servers), time.clone()).unwrap();
    assert_eq!(
        clock.last_sync().kissed,
        vec![(kisser.to_string(), *b"RATE")]
    );
    clock.resync().unwrap();
    assert_eq!(asked(&kissed), 1);
    time.advance(Duration::from_secs(127));
    clock.resync().unwrap();
    assert_eq!(asked(&kissed), 1);
    time.advance(Duration::from_secs(1));
    clock.resync().unwrap();
    assert_eq!(asked(&kissed), 2);
    assert!(clock.last_sync().kissed.is_empty());
    // The raised poll stays: the server is held off after every ask.
    time.advance(Duration::from_secs(127));
    clock.resync().unwrap();
    assert_eq!(asked(&kissed), 2);
    time.advance(Duration::from_secs(1));
    clock.resync().unwrap();
    assert_eq!(asked(&kissed), 3);
}

#[test]
fn a_deny_kiss_stops_the_server_for_good() {
    let time = Arc::new(ManualTime::new(wall()));
    let (denier, denied) = counting(vec![Answer::Kiss(b"DENY")]);
    let honest = counting_trio(vec![Answer::Ahead(0); 2]);
    let servers = [vec![denier], honest].concat();
    let clock = NtpClock::sync_with(Servers::new(servers), time.clone()).unwrap();
    assert_eq!(clock.last_sync().denied, vec![denier.to_string()]);
    time.advance(Duration::from_secs(86_400));
    clock.resync().unwrap();
    assert_eq!(asked(&denied), 1);
    // A denied server is listed on every sync, though it isn't asked.
    assert_eq!(clock.last_sync().denied, vec![denier.to_string()]);
    assert!(clock.last_sync().kissed.is_empty());
    let (alone, _) = counting(vec![Answer::Kiss(b"DENY")]);
    assert!(matches!(
        NtpClock::sync(Servers::new([alone])),
        Err(ClockError::Kiss(code)) if &code == b"DENY"
    ));
}

#[test]
fn a_name_is_resolved_again_on_every_sync() {
    let (first, asked_first) = counting(vec![Answer::Ahead(0)]);
    let (second, asked_second) = counting(vec![Answer::Ahead(0)]);
    let others = counting_trio(vec![Answer::Ahead(0); 2]);
    let address = Arc::new(Mutex::new(first));
    let resolving = address.clone();
    let servers = Servers::new(["moving.test:123", "b.test:123", "c.test:123"]).resolving_with(
        move |name: &str| {
            Ok(match name {
                "moving.test:123" => *resolving.lock().unwrap(),
                "b.test:123" => others[0],
                _ => others[1],
            })
        },
    );
    let clock = NtpClock::sync(servers).unwrap();
    *address.lock().unwrap() = second;
    clock.resync().unwrap();
    assert_eq!((asked(&asked_first), asked(&asked_second)), (1, 1));
}

#[test]
fn a_failed_resync_reports_what_it_found() {
    let servers = counting_trio(vec![Answer::Ahead(0)]);
    let clock = NtpClock::sync(Servers::new(servers.clone())).unwrap();
    assert!(clock.last_sync().unanswered.is_empty());
    assert!(matches!(clock.resync(), Err(ClockError::Io(_))));
    let names: Vec<_> = servers.iter().map(SocketAddr::to_string).collect();
    assert_eq!(clock.last_sync().unanswered, names);
}

#[test]
fn a_leap_second_a_server_announces_is_noted() {
    for (indicator, leap) in [(0, None), (1, Some(Leap::Insert)), (2, Some(Leap::Delete))] {
        let servers = counting_trio(vec![Answer::Leap(indicator)]);
        let clock = NtpClock::sync(Servers::new(servers)).unwrap();
        assert_eq!(clock.last_sync().leap, leap);
    }
}

#[test]
fn a_refresh_takes_a_sample_from_the_servers() {
    let ntp = NtpClock::sync(Servers::new(trio("127.0.0.1:0", vec![0, 50]))).unwrap();
    assert!(matches!(
        Refresh::refresh(&ntp),
        Err(ClockError::Fault(ClockFault::Drifted))
    ));
    reads_ahead(&ntp, 50);
    let ntp = NtpClock::sync(Servers::new(trio("127.0.0.1:0", vec![0, 50]))).unwrap();
    let system = SystemClock::Ntp(ntp);
    assert!(matches!(
        Refresh::refresh(&system),
        Err(ClockError::Fault(ClockFault::Drifted))
    ));
    reads_ahead(&system, 50);
}

#[test]
fn two_of_three_agreeing_servers_pass_the_sync() {
    let honest = counting_trio(vec![Answer::Ahead(0)]);
    let (wrong, _) = counting(vec![Answer::Ahead(100)]);
    let clock = NtpClock::sync(Servers::new([honest[0], honest[1], wrong])).unwrap();
    reads_ahead(&clock, 0);
    assert_eq!(clock.last_sync().falsetickers, vec![wrong.to_string()]);
}

#[test]
fn fewer_than_three_answers_fail_the_sync_and_are_reported() {
    let (_quiet, unreachable) = silent();
    let two = [
        counting(vec![Answer::Ahead(0)]).0,
        unreachable,
        counting(vec![Answer::Ahead(0)]).0,
    ];
    assert!(matches!(
        NtpClock::sync(Servers::new(two)),
        Err(ClockError::TooFewAnswers { answered: 2 })
    ));
    // Four servers, two of them gone by the resync: the two that agree
    // aren't enough, and the window stays as it was.
    let staying = counting_trio(vec![Answer::Ahead(0); 2]);
    let (leaving, _) = counting(vec![Answer::Ahead(0)]);
    let servers = [staying[0], staying[1], leaving, unreachable];
    let clock = NtpClock::sync(Servers::new(servers)).unwrap();
    let before = clock.now().unwrap();
    assert!(matches!(
        clock.resync(),
        Err(ClockError::TooFewAnswers { answered: 2 })
    ));
    assert_eq!(
        clock.last_sync().unanswered,
        vec![leaving.to_string(), unreachable.to_string()]
    );
    let after = clock.now().unwrap();
    assert!(after.revision.0 - after.settled.0 >= before.revision.0 - before.settled.0);
    assert_eq!(
        ClockError::TooFewAnswers { answered: 2 }.to_string(),
        "2 of the NTP servers answered; a sync needs 3"
    );
}
