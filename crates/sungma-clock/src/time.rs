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

/// A bracket this narrow, 1 ms, is taken without trying for a narrower
/// one.
const NARROW: u64 = 1_000_000;

/// How many brackets [`TimeSource::moment`] tries for a narrow one.
const TRIES: usize = 3;

/// Both clocks read as one moment. The system clock is read between two
/// readings of the monotonic clock, `monotonic` is their middle, and
/// `uncertainty` is how far apart they were: a thread descheduled between
/// the reads widens it rather than skewing the pair.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Moment {
    pub wall: u64,
    pub monotonic: u64,
    pub uncertainty: u64,
}

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

    /// Both clocks as one moment: the narrowest of up to three
    /// brackets, stopping at the first no wider than 1 ms.
    fn moment(&self) -> Moment {
        let mut best = bracket(self);
        for _ in 1..TRIES {
            if best.uncertainty <= NARROW {
                break;
            }
            let next = bracket(self);
            if next.uncertainty < best.uncertainty {
                best = next;
            }
        }
        best
    }
}

/// The system clock read between two readings of the monotonic clock.
fn bracket(time: &(impl TimeSource + ?Sized)) -> Moment {
    let before = time.monotonic();
    let wall = time.wall();
    let after = time.monotonic();
    let uncertainty = after.saturating_sub(before);
    Moment {
        wall,
        monotonic: before + uncertainty / 2,
        uncertainty,
    }
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
        #[cfg(target_os = "linux")]
        return boottime();
        #[cfg(not(target_os = "linux"))]
        return since_start();
    }
}

#[cfg(target_os = "linux")]
fn boottime() -> u64 {
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
fn since_start() -> u64 {
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
    use std::{sync::Mutex, thread, time::Duration};

    use super::*;

    /// A source whose monotonic clock reads from a script, one value per
    /// read, with the system clock at 1,000.
    struct Scripted(Mutex<Vec<u64>>);

    impl Scripted {
        fn new(monotonic: &[u64]) -> Self {
            Self(Mutex::new(monotonic.iter().rev().copied().collect()))
        }

        fn left(&self) -> usize {
            self.0.lock().unwrap().len()
        }
    }

    impl TimeSource for Scripted {
        fn wall(&self) -> u64 {
            1_000
        }

        fn monotonic(&self) -> u64 {
            self.0.lock().unwrap().pop().unwrap()
        }
    }

    fn moment(monotonic: u64, uncertainty: u64) -> Moment {
        Moment {
            wall: 1_000,
            monotonic,
            uncertainty,
        }
    }

    #[test]
    fn a_narrow_bracket_is_taken_at_once() {
        let time = Scripted::new(&[10, 10 + NARROW, 0, 0]);
        assert_eq!(time.moment(), moment(10 + NARROW / 2, NARROW));
        assert_eq!(time.left(), 2);
    }

    #[test]
    fn a_wide_bracket_is_tried_again_and_the_narrowest_kept() {
        let wide = 3 * NARROW;
        let time = Scripted::new(&[0, wide, 100, 100 + 2 * NARROW, 200, 200 + 2 * NARROW]);
        assert_eq!(time.moment(), moment(100 + NARROW, 2 * NARROW));
        let time = Scripted::new(&[0, wide, 100, 100 + wide, 200, 200 + 4 * NARROW]);
        assert_eq!(time.moment(), moment(wide / 2, wide));
        let time = Scripted::new(&[0, wide, 100, 100 + NARROW, 7, 7]);
        assert_eq!(time.moment(), moment(100 + NARROW / 2, NARROW));
        assert_eq!(time.left(), 2);
    }

    #[test]
    fn a_monotonic_clock_read_out_of_order_has_no_uncertainty() {
        let time = Scripted::new(&[50, 40]);
        assert_eq!(bracket(&time), moment(50, 0));
    }

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
