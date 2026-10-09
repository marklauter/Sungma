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
mod nts;
mod refresher;
mod servers;
mod system;
mod time;
mod wall;

use std::io;

pub use dev::DevClock;
#[cfg(target_os = "linux")]
pub use linux::LinuxClock;
#[cfg(feature = "test-util")]
pub use manual::ManualClock;
#[cfg(fuzzing)]
pub use ntp::fuzz_answer;
pub use ntp::{Leap, NtpClock, SyncReport};
pub use refresher::{POLL, Refresh, refresher};
pub use servers::{Resolve, Servers};
pub use system::SystemClock;
#[cfg(feature = "test-util")]
pub use time::ManualTime;
pub use time::{Moment, OsTime, TimeSource};

/// Why a clock couldn't take a sample.
#[derive(Debug, thiserror::Error)]
pub enum ClockError {
    #[error("no NTP server address")]
    NoServers,
    #[error("NTP: {0}")]
    Io(#[from] io::Error),
    #[error("NTP reply refused: {0}")]
    Reply(&'static str),
    #[error("NTP servers disagree: no range holds a majority")]
    Disagree,
    #[error("{answered} of the NTP servers answered; a sync needs 3")]
    TooFewAnswers { answered: usize },
    #[error("the NTP server sent Kiss-o'-Death {}", String::from_utf8_lossy(.0))]
    Kiss([u8; 4]),
    #[error(transparent)]
    Fault(#[from] sungma::clock::ClockFault),
    #[error("a refresh panicked: {0}")]
    Panicked(String),
    #[error("NTS: {0}")]
    Nts(String),
}
