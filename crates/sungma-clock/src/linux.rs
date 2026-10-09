//! The kernel's own bound. NTP daemons such as chrony keep the kernel's
//! `maxerror` current through `adjtimex`, and the kernel grows it by
//! 500 ppm between their updates.

use std::{
    future::Future,
    sync::{Mutex, PoisonError},
    time::Duration,
};

use libc::{c_int, c_long};
use sungma::clock::{Clock, ClockFault, Reading, Revision};

use crate::{
    ClockStatus, Moment, OsTime, TimeSource,
    status::{self, Faults},
    wall::{self, Carry, MAX_BOUND},
};

/// The kernel caps `maxerror` at 16 s, NTP's phase limit. A clock that
/// can't be read is taken to be that far off.
const PHASE_LIMIT_MICROS: u64 = 16_000_000;

/// The system clock, give or take the kernel's `maxerror`. Each reading
/// also compares the system clock with the monotonic clock since the last
/// one, to catch a step the kernel's bound doesn't cover, and carries it in
/// every later reading's bound until chrony can have corrected it.
#[derive(Debug)]
pub struct LinuxClock<T = OsTime> {
    time: T,
    last: Mutex<Last>,
    faults: Faults,
}

/// What the last reading saw: its moment, the kernel's bound, and the
/// steps the kernel's bound doesn't yet cover.
#[derive(Clone, Copy, Debug)]
struct Last {
    moment: Moment,
    kernel: u64,
    carry: Carry,
}

impl LinuxClock {
    /// `None` when the kernel's clock isn't synchronized, or its bound is
    /// wider than a write should wait.
    pub fn detect() -> Option<Self> {
        Self::detect_with(OsTime)
    }
}

impl<T: TimeSource> LinuxClock<T> {
    /// [`LinuxClock::detect`], comparing the clocks `time` reads.
    pub fn detect_with(time: T) -> Option<Self> {
        let (state, status, maxerror) = read();
        assess(state, status, maxerror, 0).ok()?;
        let last = Mutex::new(Last {
            moment: time.moment(),
            kernel: bound(state, maxerror),
            carry: Carry::default(),
        });
        Some(Self {
            time,
            last,
            faults: Faults::default(),
        })
    }

    /// The clock's state, read from the kernel without counting a fault or
    /// disturbing jump detection.
    pub fn status(&self) -> ClockStatus {
        let (state, status, maxerror) = read();
        let last = *self.last.lock().unwrap_or_else(PoisonError::into_inner);
        let carried = last.carry.at(self.time.monotonic());
        let reading = assess(state, status, maxerror, carried)
            .and_then(|bound| wall::reading(self.time.wall(), bound));
        ClockStatus::of(reading, self.faults.counts())
    }

    fn reading(&self) -> Result<Reading, ClockFault> {
        let (state, status, maxerror) = read();
        let kernel = bound(state, maxerror);
        let now = self.time.moment();
        // Two threads may store `last` out of order, leaving the older
        // reading behind. That's harmless: every stored reading is
        // consistent, and its steps are carried either way.
        let (step, carried) = {
            let mut last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
            let then = last.moment;
            let moved = i128::from(now.wall) - i128::from(then.wall);
            let elapsed = Duration::from_nanos(now.monotonic.saturating_sub(then.monotonic));
            let slack = then.uncertainty.saturating_add(now.uncertainty);
            // On Linux the monotonic clock is slewed with the system clock, so
            // only a step parts them.
            let step = wall::step(moved - wall::nanos(elapsed), slack, 0);
            let dropped = kernel < last.kernel;
            let carry = last
                .carry
                .next(wall::stepped(step, slack), dropped, now.monotonic);
            *last = Last {
                moment: now,
                kernel,
                carry,
            };
            (step.map(|_| slack), carry.at(now.monotonic))
        };
        let slack = step?;
        let widen = carried.saturating_add(slack);
        wall::reading(now.wall, assess(state, status, maxerror, widen)?)
    }
}

/// The kernel clock's state, status and `maxerror` in microseconds.
fn read() -> (c_int, c_int, c_long) {
    // SAFETY: `timex` holds only integers, so all zeros is a valid value.
    let mut timex: libc::timex = unsafe { std::mem::zeroed() };
    // SAFETY: modes is 0, so `adjtimex` only reads, into a `timex` it may
    // write.
    let state = unsafe { libc::adjtimex(&mut timex) };
    (state, timex.status, timex.maxerror)
}

fn synchronized(state: c_int, status: c_int) -> bool {
    state != -1 && state != libc::TIME_ERROR && status & libc::STA_UNSYNC == 0
}

/// `maxerror` in nanoseconds, or the phase limit when it can't be read.
fn bound(state: c_int, maxerror: c_long) -> u64 {
    let micros = match state {
        -1 => PHASE_LIMIT_MICROS,
        _ => u64::try_from(maxerror).unwrap_or(PHASE_LIMIT_MICROS),
    };
    micros.saturating_mul(1_000)
}

/// The bound, widened by `step` nanoseconds the system clock moved against
/// the monotonic clock, or why the clock can't be trusted.
fn assess(state: c_int, status: c_int, maxerror: c_long, step: u64) -> Result<u64, ClockFault> {
    let bound = bound(state, maxerror).saturating_add(step);
    if !synchronized(state, status) || bound > MAX_BOUND {
        return Err(ClockFault::Unsynchronized);
    }
    Ok(bound)
}

impl<T: TimeSource> Clock for LinuxClock<T> {
    fn now(&self) -> Result<Reading, ClockFault> {
        self.faults.record(self.reading())
    }

    fn wait(&self, stamped: Reading) -> impl Future<Output = Result<(), ClockFault>> + Send {
        wall::wait(self, stamped)
    }

    fn observe(&self, seen: Revision) -> Result<(), ClockFault> {
        let now = self.now()?;
        self.faults.record(status::behind(now, seen))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clock_is_synchronized_unless_the_kernel_says_otherwise() {
        assert!(synchronized(libc::TIME_OK, 0));
        assert!(!synchronized(-1, 0));
        assert!(!synchronized(libc::TIME_ERROR, 0));
        assert!(!synchronized(libc::TIME_OK, libc::STA_UNSYNC));
        assert!(synchronized(libc::TIME_OK, libc::STA_PLL));
    }

    #[test]
    fn the_bound_is_maxerror_unless_it_cannot_be_read() {
        assert_eq!(bound(libc::TIME_OK, 1_500), 1_500_000);
        assert_eq!(bound(-1, 1_500), 16_000_000_000);
        assert_eq!(bound(libc::TIME_OK, -1), 16_000_000_000);
    }

    #[test]
    fn a_step_widens_the_bound_and_an_unsynchronized_clock_faults() {
        assert_eq!(assess(libc::TIME_OK, 0, 1_500, 7), Ok(1_500_007));
        assert_eq!(assess(libc::TIME_OK, 0, 1_000_000, 0), Ok(1_000_000_000));
        assert_eq!(
            assess(libc::TIME_OK, 0, 1_000_000, 1),
            Err(ClockFault::Unsynchronized)
        );
        assert_eq!(
            assess(libc::TIME_OK, libc::STA_UNSYNC, 1_500, 0),
            Err(ClockFault::Unsynchronized)
        );
    }

    #[test]
    fn every_reading_after_a_step_contains_true_time() {
        let start = wall::wall_nanos();
        let time = std::sync::Arc::new(crate::ManualTime::new(start));
        if let Some(clock) = LinuxClock::detect_with(time.clone()) {
            // The system clock jumps 8 ms ahead; true time doesn't.
            time.step(8_000_000);
            for _ in 0..3 {
                time.advance(Duration::from_millis(1));
                let truth = time.wall() - 8_000_000;
                let now = clock.now().unwrap();
                assert!(now.settled.0 <= truth && truth <= now.revision.0, "{now:?}");
            }
            // A jump past 10 ms faults once, and stays in the bound after.
            time.step(20_000_000);
            assert_eq!(clock.now(), Err(ClockFault::Jumped { by: 20_000_000 }));
            let truth = time.wall() - 28_000_000;
            let now = clock.now().unwrap();
            assert!(now.settled.0 <= truth && truth <= now.revision.0, "{now:?}");
            let status = clock.status();
            assert!(status.width.is_some_and(|width| width >= 56_000_000));
        }
    }

    #[test]
    fn a_clock_is_detected_when_the_kernel_is_synchronized() {
        let (state, status, maxerror) = read();
        let trusted = assess(state, status, maxerror, 0).is_ok();
        assert_eq!(LinuxClock::detect().is_some(), trusted);
    }

    #[test]
    fn a_step_of_the_system_clock_faults_a_detected_clock() {
        let time = std::sync::Arc::new(crate::ManualTime::new(wall::wall_nanos()));
        if let Some(clock) = LinuxClock::detect_with(time.clone()) {
            time.advance(Duration::from_millis(1));
            assert!(clock.now().is_ok());
            time.step(20_000_000);
            assert_eq!(clock.now(), Err(ClockFault::Jumped { by: 20_000_000 }));
            time.advance(Duration::from_millis(1));
            assert!(clock.now().is_ok());
        }
    }

    #[test]
    fn a_status_reads_the_kernel_without_disturbing_jump_detection() {
        let time = std::sync::Arc::new(crate::ManualTime::new(wall::wall_nanos()));
        if let Some(clock) = LinuxClock::detect_with(time.clone()) {
            time.step(20_000_000);
            let status = clock.status();
            assert!(status.width.is_some_and(|width| width > 0));
            assert_eq!((status.fault, status.faults.jumped), (None, 0));
            // The status didn't take the moment, so the step still shows.
            assert_eq!(clock.now(), Err(ClockFault::Jumped { by: 20_000_000 }));
            assert_eq!(clock.status().faults.jumped, 1);
            assert!(clock.observe(Revision(u64::MAX)).is_err());
            assert_eq!(clock.status().faults.behind, 1);
        }
    }

    #[tokio::test]
    async fn a_detected_clock_reads_the_system_clock_and_waits_it_out() {
        if let Some(clock) = LinuxClock::detect() {
            let before = wall::wall_nanos();
            let now = clock.now().unwrap();
            let after = wall::wall_nanos();
            assert!(now.settled.0 <= after && before <= now.revision.0);
            assert!((1..=2 * MAX_BOUND).contains(&(now.revision.0 - now.settled.0)));
            let waited = tokio::time::timeout(Duration::from_secs(5), clock.wait(now)).await;
            waited.unwrap().unwrap();
            assert!(clock.now().unwrap().settled > now.revision);
        }
    }
}
