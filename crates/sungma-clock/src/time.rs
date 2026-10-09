//! The machine's two clocks, behind a trait so a test can drive them.

use std::sync::Arc;
#[cfg(not(target_os = "linux"))]
use std::{sync::OnceLock, time::Instant};
#[cfg(feature = "test-util")]
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use crate::wall;

/// A system clock and a monotonic clock. The clocks that read time take
/// both from a source, [`OsTime`] in production.
pub trait TimeSource: Send + Sync {
    /// The system clock, in nanoseconds since the Unix epoch. A time
    /// service may step it.
    fn wall(&self) -> u64;

    /// A clock that never steps, in nanoseconds from an arbitrary start.
    /// It keeps counting while the machine is suspended where the platform
    /// allows, as Linux's `CLOCK_BOOTTIME` does.
    fn monotonic(&self) -> u64;
}

impl<T: TimeSource + ?Sized> TimeSource for Arc<T> {
    fn wall(&self) -> u64 {
        (**self).wall()
    }

    fn monotonic(&self) -> u64 {
        (**self).monotonic()
    }
}

/// The operating system's clocks. The monotonic clock is `CLOCK_BOOTTIME`
/// on Linux, and [`std::time::Instant`] elsewhere.
#[derive(Clone, Copy, Debug, Default)]
pub struct OsTime;

impl TimeSource for OsTime {
    fn wall(&self) -> u64 {
        wall::wall_nanos()
    }

    fn monotonic(&self) -> u64 {
        monotonic()
    }
}

#[cfg(target_os = "linux")]
fn monotonic() -> u64 {
    // SAFETY: `timespec` holds only integers, so all zeros is a valid value.
    let mut now: libc::timespec = unsafe { std::mem::zeroed() };
    // SAFETY: `now` is a valid `timespec` for `clock_gettime` to write.
    let failed = unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut now) };
    assert_eq!(failed, 0, "every supported kernel has CLOCK_BOOTTIME");
    let seconds = u64::try_from(now.tv_sec).unwrap_or(0);
    let nanos = u64::try_from(now.tv_nsec).unwrap_or(0);
    seconds.saturating_mul(1_000_000_000).saturating_add(nanos)
}

#[cfg(not(target_os = "linux"))]
fn monotonic() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    let since = START.get_or_init(Instant::now).elapsed();
    u64::try_from(since.as_nanos()).unwrap_or(u64::MAX)
}

/// Clocks a test moves by hand: time passing moves both, and a step moves
/// the system clock alone.
#[cfg(feature = "test-util")]
#[derive(Debug, Default)]
pub struct ManualTime {
    wall: AtomicU64,
    monotonic: AtomicU64,
}

#[cfg(feature = "test-util")]
impl ManualTime {
    /// Clocks whose system clock reads `wall`.
    pub fn new(wall: u64) -> Self {
        Self {
            wall: AtomicU64::new(wall),
            monotonic: AtomicU64::new(0),
        }
    }

    /// Time passes: both clocks move `by`.
    pub fn advance(&self, by: Duration) {
        let by = u64::try_from(by.as_nanos()).unwrap_or(u64::MAX);
        self.wall.fetch_add(by, Ordering::SeqCst);
        self.monotonic.fetch_add(by, Ordering::SeqCst);
    }

    /// A time service steps the system clock alone, `by` nanoseconds
    /// forward, or back when negative.
    pub fn step(&self, by: i64) {
        // Adding the two's complement of a negative step subtracts it.
        self.wall.fetch_add(by.cast_unsigned(), Ordering::SeqCst);
    }
}

#[cfg(feature = "test-util")]
impl TimeSource for ManualTime {
    fn wall(&self) -> u64 {
        self.wall.load(Ordering::SeqCst)
    }

    fn monotonic(&self) -> u64 {
        self.monotonic.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use std::{thread, time::Duration};

    use super::*;

    #[test]
    fn the_os_clocks_both_count_time_passing() {
        let (wall, monotonic) = (OsTime.wall(), OsTime.monotonic());
        thread::sleep(Duration::from_millis(20));
        let walled = OsTime.wall() - wall;
        let counted = OsTime.monotonic() - monotonic;
        assert!((20_000_000..1_000_000_000).contains(&counted), "{counted}");
        assert!(walled.abs_diff(counted) < 10_000_000, "{walled} {counted}");
        assert!(OsTime.wall().abs_diff(wall::wall_nanos()) < 1_000_000_000);
    }

    #[test]
    fn a_shared_source_reads_the_source() {
        let time = Arc::new(ManualTime::new(5));
        time.advance(Duration::from_nanos(3));
        assert_eq!((time.wall(), TimeSource::monotonic(&time)), (8, 3));
    }

    #[test]
    fn manual_time_passes_on_both_clocks_and_steps_on_one() {
        let time = ManualTime::new(1_000);
        assert_eq!((time.wall(), time.monotonic()), (1_000, 0));
        time.advance(Duration::from_nanos(50));
        assert_eq!((time.wall(), time.monotonic()), (1_050, 50));
        time.step(25);
        assert_eq!((time.wall(), time.monotonic()), (1_075, 50));
        time.step(-75);
        assert_eq!((time.wall(), time.monotonic()), (1_000, 50));
    }
}
