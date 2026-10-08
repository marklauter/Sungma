//! The revision clock, which orders writes.
//!
//! A revision is a reading of the clock. A [`Clock`] never knows true time,
//! only an [`Interval`] that contains it. A writer stamps its write with
//! [`Interval::latest`], and acknowledges only once [`Clock::wait_past`] has
//! seen [`Interval::earliest`] pass the stamp. Any write that starts after
//! the acknowledgement then gets a later revision, wherever it runs.

/// Each write produces the next revision, and a read at a revision sees
/// every fact written at or before it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Revision(pub u64);

/// Revisions between which true time lies.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Interval {
    pub earliest: Revision,
    pub latest: Revision,
}

/// Reads the revision clock. Implementations live in their own crates;
/// the writer ports call one inside a commit, and the domain never does.
pub trait Clock {
    /// An interval that contains true time.
    fn now(&self) -> Interval;

    /// Resolves once [`Clock::now`] reports an `earliest` past `revision`.
    fn wait_past(&self, revision: Revision) -> impl Future<Output = ()> + Send;
}
