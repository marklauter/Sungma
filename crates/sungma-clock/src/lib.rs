//! Implementations of [`sungma::clock::Clock`].
//!
//! [`SystemClock`] is the production clock: the Linux kernel's own bound
//! where it reports one, and [`NtpClock`] everywhere else, Windows
//! included.
//! [`DevClock`] is a counter for a single node, and `ManualClock`, behind
//! the `test-util` feature, is set by hand.
//!
//! The clocks that read time report revisions in nanoseconds since the Unix
//! epoch.

mod dev;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(feature = "test-util")]
mod manual;
mod ntp;
mod system;
mod wall;

use std::io;

pub use dev::DevClock;
#[cfg(target_os = "linux")]
pub use linux::LinuxClock;
#[cfg(feature = "test-util")]
pub use manual::ManualClock;
pub use ntp::NtpClock;
pub use system::SystemClock;

/// Why a clock couldn't take a sample.
#[derive(Debug, thiserror::Error)]
pub enum ClockError {
    #[error("no NTP server address")]
    NoServers,
    #[error("NTP: {0}")]
    Io(#[from] io::Error),
    #[error("NTP reply refused: {0}")]
    Reply(&'static str),
    #[error("NTP servers disagree: their ranges don't overlap")]
    Disagree,
    #[error(transparent)]
    Fault(#[from] sungma::clock::ClockFault),
}
