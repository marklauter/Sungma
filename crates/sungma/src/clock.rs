//! The revision clock, which orders writes.
//!
//! A [`Revision`] is a reading of the clock. A [`Clock`] never knows true
//! time, only a [`Reading`] that brackets it. A writer stamps its write
//! with the reading's `revision`, and acknowledges only once
//! [`Clock::wait`] has seen the clock's `settled` time pass the stamp. Any
//! write that starts after the acknowledgement then gets a later revision,
//! wherever it runs.
//!
//! A clock that finds its bound can't be trusted reports a [`ClockFault`]
//! instead of a reading. The node refuses to stamp until the clock
//! recovers, and another node takes the write.

/// Each write produces the next revision, and a read at a revision sees
/// every fact written at or before it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Revision(pub u64);

/// One reading of the clock, which brackets true time.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Reading {
    /// True time is certainly past this.
    pub settled: Revision,
    /// True time is not yet past this. A write is stamped with it.
    pub revision: Revision,
}

/// Why a clock can't give a reading it trusts.
#[derive(Clone, Copy, PartialEq, Eq, Debug, thiserror::Error)]
pub enum ClockFault {
    /// The system clock stepped `by` nanoseconds against the monotonic
    /// clock, as when a virtual machine is paused or moved.
    #[error("the system clock jumped by {by} ns")]
    Jumped { by: i64 },
    /// A new sample doesn't overlap the earlier ones, so the clock drifted
    /// faster than its allowance.
    #[error("the clock drifted faster than its allowance")]
    Drifted,
    /// Another node stamped `seen`, which this clock reads as not yet
    /// arrived.
    #[error("revision {seen:?} was stamped elsewhere, but this clock reads {revision:?}")]
    Behind { seen: Revision, revision: Revision },
    /// The clock isn't synchronized, or its bound grew past the most a
    /// write should wait.
    #[error("the clock isn't synchronized")]
    Unsynchronized,
}

/// Reads the revision clock. Implementations live in their own crates;
/// the writer ports call one inside a commit, and the domain never does.
pub trait Clock {
    /// A reading that brackets true time.
    fn now(&self) -> Result<Reading, ClockFault>;

    /// Resolves once the clock's `settled` time is past `stamped.revision`,
    /// or with the fault that stopped the wait.
    fn wait(&self, stamped: Reading) -> impl Future<Output = Result<(), ClockFault>> + Send;

    /// Checks a revision another node stamped, such as one a revision
    /// token carries. A revision past this clock's `revision` is one true
    /// time can't have reached yet, so one of the two clocks is wrong.
    fn observe(&self, seen: Revision) -> Result<(), ClockFault> {
        behind(self.now()?, seen)
    }
}

/// [`ClockFault::Behind`] when another node stamped `seen` past the
/// `revision` of the reading `now`, the check [`Clock::observe`] makes.
pub fn behind(now: Reading, seen: Revision) -> Result<(), ClockFault> {
    if seen > now.revision {
        return Err(ClockFault::Behind {
            seen,
            revision: now.revision,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_revision_past_the_readings_is_behind() {
        let now = Reading {
            settled: Revision(5),
            revision: Revision(9),
        };
        assert_eq!(behind(now, Revision(9)), Ok(()));
        assert_eq!(
            behind(now, Revision(10)),
            Err(ClockFault::Behind {
                seen: Revision(10),
                revision: Revision(9),
            })
        );
    }
}
