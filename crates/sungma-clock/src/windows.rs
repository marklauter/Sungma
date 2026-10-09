//! The Windows Time service's bound. Windows has no API that reports it,
//! so this reads `w32tm /query /status` and grows the bound by drift until
//! the next read.

use std::{future::Future, process::Command};

use sungma::clock::{Clock, ClockFault, Reading};

use crate::{
    ClockError, w32tm,
    wall::{self, MAX_BOUND, Sample, Sampled},
};

/// The system clock, give or take what the Windows Time service last
/// reported.
#[derive(Debug)]
pub struct WindowsClock(Sampled);

impl WindowsClock {
    /// `None` when the service isn't running, isn't synchronized, reports
    /// a bound wider than a write should wait, or reports in a language
    /// other than English.
    pub fn detect() -> Option<Self> {
        query()
            .filter(|sample| sample.bound <= MAX_BOUND)
            .map(|sample| Self(Sampled::new(sample)))
    }

    /// Reads the service's status again, into the window. When it can't,
    /// the window stays as it was and its bounds keep growing.
    pub fn refresh(&self) -> Result<(), ClockError> {
        Ok(self.0.add(query().ok_or(ClockError::Unsynchronized)?)?)
    }
}

fn query() -> Option<Sample> {
    let output = Command::new("w32tm")
        .args(["/query", "/status"])
        .output()
        .ok()?;
    let status = String::from_utf8_lossy(&output.stdout);
    let bound = w32tm::bound(&status).filter(|_| output.status.success())?;
    Some(Sample { offset: 0, bound })
}

impl Clock for WindowsClock {
    fn now(&self) -> Result<Reading, ClockFault> {
        self.0.now()
    }

    fn wait(&self, stamped: Reading) -> impl Future<Output = Result<(), ClockFault>> + Send {
        wall::wait(self, stamped)
    }
}
