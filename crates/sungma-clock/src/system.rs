//! The production clock, chosen at startup.

use std::future::Future;

use crate::{Clock, ClockFault, Reading};
use sungma::revision::Revision;

#[cfg(target_os = "linux")]
use crate::LinuxClock;
use crate::{ClockError, ClockStatus, NtpClock, Servers, wall};

/// The Linux kernel's clock when it reports a bound, else NTP. An enum
/// rather than a `Box<dyn Clock>`: [`Clock::wait`] returns `impl Future`,
/// so the trait can't be a trait object, and the Linux variant exists only
/// on Linux.
#[derive(Debug)]
#[cfg_attr(
    target_os = "linux",
    expect(
        clippy::large_enum_variant,
        reason = "one per process, made once and never moved on a hot path"
    )
)]
pub enum SystemClock {
    #[cfg(target_os = "linux")]
    Linux(LinuxClock),
    Ntp(NtpClock),
}

impl SystemClock {
    /// The Linux kernel's clock when it is synchronized, else an
    /// [`NtpClock`] synced from `servers`, which blocks on the network.
    pub fn detect(servers: Servers) -> Result<Self, ClockError> {
        #[cfg(target_os = "linux")]
        if let Some(clock) = LinuxClock::detect() {
            return Ok(Self::Linux(clock));
        }
        NtpClock::sync(servers).map(Self::Ntp)
    }

    /// The clock's state, read without disturbing it.
    pub fn status(&self) -> ClockStatus {
        match self {
            #[cfg(target_os = "linux")]
            Self::Linux(clock) => clock.status(),
            Self::Ntp(clock) => clock.status(),
        }
    }

    /// Takes a fresh sample, for a clock that samples. The kernel keeps
    /// Linux's bound current, so it needs none.
    pub fn refresh(&self) -> Result<(), ClockError> {
        match self {
            #[cfg(target_os = "linux")]
            Self::Linux(_) => Ok(()),
            Self::Ntp(clock) => clock.resync(),
        }
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Result<Reading, ClockFault> {
        match self {
            #[cfg(target_os = "linux")]
            Self::Linux(clock) => clock.now(),
            Self::Ntp(clock) => clock.now(),
        }
    }

    fn wait(&self, stamped: Reading) -> impl Future<Output = Result<(), ClockFault>> + Send {
        wall::wait(self, stamped)
    }

    fn observe(&self, seen: Revision) -> Result<(), ClockFault> {
        match self {
            #[cfg(target_os = "linux")]
            Self::Linux(clock) => clock.observe(seen),
            Self::Ntp(clock) => clock.observe(seen),
        }
    }
}
