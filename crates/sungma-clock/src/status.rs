//! What a clock reports about itself, for metrics and alerts. Exporting
//! them is left to the service that runs the clock.

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use crate::{ClockFault, Reading};

use crate::{Drift, SyncReport};

/// How often a clock has reported each fault since it started.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct FaultCounts {
    pub jumped: u64,
    pub drifted: u64,
    pub behind: u64,
    pub unsynchronized: u64,
}

/// A clock's state, read without disturbing it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ClockStatus {
    /// The reading's width, `revision` − `settled`, when the clock gives
    /// one. Commit-wait takes about this long.
    pub width: Option<u64>,
    /// The fault the clock would report now.
    pub fault: Option<ClockFault>,
    /// The faults reported since the clock started.
    pub faults: FaultCounts,
    /// Monotonic time since the newest sample, for a clock that samples.
    pub since_sample: Option<Duration>,
    /// Syncs that failed since the clock started.
    pub failed_syncs: u64,
    /// What the last sync found, for an NTP clock.
    pub last_sync: Option<SyncReport>,
    /// The drift the window measures, for a clock that samples.
    pub drift: Option<Drift>,
}

impl ClockStatus {
    /// The status of a clock that reads `reading`, with nothing else to say.
    pub(crate) fn of(reading: Result<Reading, ClockFault>, faults: FaultCounts) -> Self {
        Self {
            width: reading.ok().map(|now| now.revision.0 - now.settled.0),
            fault: reading.err(),
            faults,
            since_sample: None,
            failed_syncs: 0,
            last_sync: None,
            drift: None,
        }
    }
}

/// Counts the faults a clock reports.
#[derive(Debug, Default)]
pub(crate) struct Faults {
    jumped: AtomicU64,
    drifted: AtomicU64,
    behind: AtomicU64,
    unsynchronized: AtomicU64,
}

impl Faults {
    /// Counts `result`'s fault, if it has one, and hands it back.
    pub(crate) fn record<T>(&self, result: Result<T, ClockFault>) -> Result<T, ClockFault> {
        if let Err(fault) = &result {
            let counter = match fault {
                ClockFault::Jumped { .. } => &self.jumped,
                ClockFault::Drifted => &self.drifted,
                ClockFault::Behind { .. } => &self.behind,
                ClockFault::Unsynchronized => &self.unsynchronized,
            };
            counter.fetch_add(1, Ordering::Relaxed);
        }
        result
    }

    pub(crate) fn counts(&self) -> FaultCounts {
        FaultCounts {
            jumped: self.jumped.load(Ordering::Relaxed),
            drifted: self.drifted.load(Ordering::Relaxed),
            behind: self.behind.load(Ordering::Relaxed),
            unsynchronized: self.unsynchronized.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use sungma::revision::Revision;

    use super::*;

    fn reading(settled: u64, revision: u64) -> Reading {
        Reading {
            settled: Revision(settled),
            revision: Revision(revision),
        }
    }

    #[test]
    fn each_fault_is_counted_by_its_kind() {
        let faults = Faults::default();
        let fault = |fault| faults.record::<()>(Err(fault));
        fault(ClockFault::Jumped { by: 1 }).unwrap_err();
        fault(ClockFault::Jumped { by: 2 }).unwrap_err();
        fault(ClockFault::Drifted).unwrap_err();
        fault(ClockFault::Behind {
            seen: Revision(2),
            revision: Revision(1),
        })
        .unwrap_err();
        fault(ClockFault::Unsynchronized).unwrap_err();
        assert_eq!(faults.record(Ok(7)), Ok(7));
        assert_eq!(
            faults.counts(),
            FaultCounts {
                jumped: 2,
                drifted: 1,
                behind: 1,
                unsynchronized: 1,
            }
        );
    }

    #[test]
    fn a_status_gives_the_width_or_the_fault() {
        let counts = FaultCounts::default();
        let status = ClockStatus::of(Ok(reading(10, 25)), counts);
        assert_eq!((status.width, status.fault), (Some(15), None));
        let status = ClockStatus::of(Err(ClockFault::Drifted), counts);
        assert_eq!(
            (status.width, status.fault),
            (None, Some(ClockFault::Drifted))
        );
    }
}
