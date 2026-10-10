//! The platform's own clock on a real machine. Ignored by default: the
//! weekly platform workflow runs them on a Linux runner with chrony synced.

/// With chrony synced, the kernel reports a bound, so the Linux clock is
/// detected and reads the system clock within it. Prints the bound, to show
/// what a runner's virtual machine really gets.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "needs chrony synced"]
fn a_synced_kernel_gives_a_linux_clock() {
    use std::time::{SystemTime, UNIX_EPOCH};

    use sungma_clock::Clock;
    use sungma_clock::{LinuxClock, SystemClock};

    let wall = || {
        let since = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        u64::try_from(since.as_nanos()).unwrap()
    };
    let clock = LinuxClock::detect().expect("chrony has synced the kernel");
    let (before, now, after) = (wall(), clock.now().unwrap(), wall());
    assert!(now.settled.0 <= after && before <= now.revision.0);
    let status = clock.status();
    println!(
        "kernel bound: ±{} µs",
        (now.revision.0 - now.settled.0) / 2_000
    );
    println!("{status:?}");
    let system = SystemClock::detect(sungma_clock::Servers::new(Vec::<String>::new())).unwrap();
    assert!(matches!(system, SystemClock::Linux(_)));
}
