//! The kernel's own bound. NTP daemons such as chrony keep the kernel's
//! `maxerror` current through `adjtimex`, and the kernel grows it by
//! 500 ppm between their updates.

use std::{
    future::Future,
    sync::{Mutex, PoisonError},
    time::Duration,
};

use libc::{c_int, c_long};
use sungma::clock::{Clock, ClockFault, Reading};

use crate::{
    Moment, OsTime, TimeSource,
    wall::{self, MAX_BOUND},
};

/// The kernel caps `maxerror` at 16 s, NTP's phase limit. A clock that
/// can't be read is taken to be that far off.
const PHASE_LIMIT_MICROS: u64 = 16_000_000;

/// The system clock, give or take the kernel's `maxerror`. Each reading
/// also compares the system clock with the monotonic clock since the last
/// one, to catch a step the kernel's bound doesn't cover.
#[derive(Debug)]
pub struct LinuxClock<T = OsTime> {
    time: T,
    last: Mutex<Moment>,
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
        let last = Mutex::new(time.moment());
        Some(Self { time, last })
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
        let (state, status, maxerror) = read();
        let now = self.time.moment();
        // Two threads may swap `last` out of order, leaving the older
        // moment behind. That's harmless: every stored moment is a
        // consistent pair, and the next reading compares against one.
        let then = std::mem::replace(
            &mut *self.last.lock().unwrap_or_else(PoisonError::into_inner),
            now,
        );
        let moved = i128::from(now.wall) - i128::from(then.wall);
        let elapsed = Duration::from_nanos(now.monotonic.saturating_sub(then.monotonic));
        let slack = then.uncertainty.saturating_add(now.uncertainty);
        let step = wall::step(moved - wall::nanos(elapsed), slack)?;
        wall::reading(now.wall, assess(state, status, maxerror, step)?)
    }

    fn wait(&self, stamped: Reading) -> impl Future<Output = Result<(), ClockFault>> + Send {
        wall::wait(self, stamped)
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
