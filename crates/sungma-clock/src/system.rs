//! The production clock, chosen at startup.

use std::{future::Future, net::ToSocketAddrs};

use sungma::clock::{Clock, Reading};

#[cfg(target_os = "linux")]
use crate::LinuxClock;
#[cfg(windows)]
use crate::WindowsClock;
use crate::{ClockError, NtpClock, wall};

/// The platform's clock when it reports a bound, else NTP. An enum rather
/// than a `Box<dyn Clock>`: [`Clock::wait`] returns `impl Future`, so
/// the trait can't be a trait object, and each variant exists only on the
/// platforms that have it.
#[derive(Debug)]
pub enum SystemClock {
    #[cfg(target_os = "linux")]
    Linux(LinuxClock),
    #[cfg(windows)]
    Windows(WindowsClock),
    Ntp(NtpClock),
}

impl SystemClock {
    /// The platform's clock when it is synchronized, else an [`NtpClock`]
    /// synced from `servers`, which blocks on the network.
    pub fn detect<A: ToSocketAddrs>(servers: &[A]) -> Result<Self, ClockError> {
        #[cfg(target_os = "linux")]
        if let Some(clock) = LinuxClock::detect() {
            return Ok(Self::Linux(clock));
        }
        #[cfg(windows)]
        if let Some(clock) = WindowsClock::detect() {
            return Ok(Self::Windows(clock));
        }
        NtpClock::sync(servers).map(Self::Ntp)
    }

    /// Takes a fresh sample, for a clock that samples. The kernel keeps
    /// Linux's bound current, so it needs none.
    pub fn refresh(&self) -> Result<(), ClockError> {
        match self {
            #[cfg(target_os = "linux")]
            Self::Linux(_) => Ok(()),
            #[cfg(windows)]
            Self::Windows(clock) => clock.refresh(),
            Self::Ntp(clock) => clock.resync(),
        }
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Reading {
        match self {
            #[cfg(target_os = "linux")]
            Self::Linux(clock) => clock.now(),
            #[cfg(windows)]
            Self::Windows(clock) => clock.now(),
            Self::Ntp(clock) => clock.now(),
        }
    }

    fn wait(&self, stamped: Reading) -> impl Future<Output = ()> + Send {
        wall::wait(self, stamped)
    }
}
