//! The kernel's own bound. NTP daemons such as chrony keep the kernel's
//! `maxerror` current through `adjtimex`, and the kernel grows it by
//! 500 ppm between their updates.

use std::future::Future;

use libc::{c_int, c_long};
use sungma::clock::{Clock, Reading};

use crate::wall;

/// The kernel caps `maxerror` at 16 s, NTP's phase limit. A clock that
/// can't be read is taken to be that far off.
const PHASE_LIMIT_MICROS: u64 = 16_000_000;

/// The system clock, give or take the kernel's `maxerror`.
#[derive(Debug)]
pub struct LinuxClock(());

impl LinuxClock {
    /// `None` when the kernel's clock isn't synchronized.
    pub fn detect() -> Option<Self> {
        let (state, status, _) = read();
        synchronized(state, status).then_some(Self(()))
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

impl Clock for LinuxClock {
    fn now(&self) -> Reading {
        let (state, _, maxerror) = read();
        wall::reading(wall::wall_nanos(), bound(state, maxerror))
    }

    fn wait(&self, stamped: Reading) -> impl Future<Output = ()> + Send {
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
    fn a_clock_is_detected_when_the_kernel_is_synchronized() {
        let (state, status, _) = read();
        assert_eq!(LinuxClock::detect().is_some(), synchronized(state, status));
    }

    #[test]
    fn a_detected_clock_reads_the_system_clock_within_its_bound() {
        if let Some(clock) = LinuxClock::detect() {
            let before = wall::wall_nanos();
            let now = clock.now();
            let after = wall::wall_nanos();
            assert!(now.settled.0 <= after && before <= now.revision.0);
            assert!((1..=2 * 16_000_000_000).contains(&(now.revision.0 - now.settled.0)));
        }
    }
}
