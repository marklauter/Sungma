//! Implementations of [`Clock`].
//!
//! [`SystemClock`] is the production clock: the Linux kernel's own bound
//! where it reports one, and [`NtpClock`] everywhere else, Windows
//! included.
//! [`DevClock`] is a counter for a single node, and `ManualClock`, behind
//! the `test-util` feature, is set by hand.
//!
//! The clocks that read time report revisions in nanoseconds since the Unix
//! epoch.

mod clock;
mod dev;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(feature = "test-util")]
mod manual;
mod ntp;
mod nts;
mod refresher;
mod servers;
mod status;
mod system;
mod time;
mod wall;

use std::io;

pub use clock::{Clock, ClockFault, Reading, behind};
pub use dev::DevClock;
#[cfg(target_os = "linux")]
pub use linux::LinuxClock;
#[cfg(feature = "test-util")]
pub use manual::ManualClock;
#[cfg(fuzzing)]
pub use ntp::fuzz_answer;
#[cfg(fuzzing)]
pub use nts::{fuzz_fields, fuzz_ke_response, fuzz_verify};
pub use ntp::{Leap, NtpClock, SyncReport};
pub use refresher::{POLL, Refresh, refresher};
pub use servers::{Resolve, Servers};
pub use status::{ClockStatus, FaultCounts};
pub use system::SystemClock;
#[cfg(feature = "test-util")]
pub use time::ManualTime;
pub use time::{Moment, OsTime, TimeSource};
pub use wall::Drift;

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
    Fault(#[from] ClockFault),
    #[error("a refresh panicked: {0}")]
    Panicked(String),
    #[error("NTS: {0}")]
    Nts(#[from] NtsError),
}

/// Why an NTS key exchange or an authenticated query failed.
#[derive(Clone, PartialEq, Debug, thiserror::Error)]
pub enum NtsError {
    #[error("the key exchange response is too long")]
    ResponseTooLong,
    /// The server sent an error record, with its code when the record
    /// carries one.
    #[error(
        "the key exchange server refused{}",
        .code.map_or_else(String::new, |code| format!(" with error code {code}"))
    )]
    Refused { code: Option<u16> },
    #[error("record type {0} is critical, but isn't known")]
    UnknownCritical(u16),
    #[error("a server name isn't text")]
    ServerName,
    #[error("the server didn't agree NTPv4 with AES-SIV")]
    Disagreed,
    #[error("the server gave no cookies")]
    NoCookies,
    #[error("{0:?} isn't a valid host name")]
    HostName(String),
    #[error("a trusted certificate isn't valid: {0}")]
    Certificate(rustls::Error),
    #[error("TLS: {0}")]
    Tls(#[from] rustls::Error),
    #[error("no cookie is left")]
    NoCookieLeft,
    /// An `NTSN` Kiss-o'-Death: the server no longer knows our cookies.
    #[error("the server no longer knows our cookies")]
    Nak,
}
