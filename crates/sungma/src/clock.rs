//! The revision clock, which orders writes.
//!
//! A [`Revision`] is a reading of the clock. A [`Clock`] never knows true
//! time, only a [`Reading`] that brackets it. A writer stamps its write
//! with the reading's `revision`, and acknowledges only once
//! [`Clock::wait`] has seen the clock's `settled` time pass the stamp. Any
//! write that starts after the acknowledgement then gets a later revision,
//! wherever it runs.

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

/// Reads the revision clock. Implementations live in their own crates;
/// the writer ports call one inside a commit, and the domain never does.
pub trait Clock {
    /// A reading that brackets true time.
    fn now(&self) -> Reading;

    /// Resolves once the clock's `settled` time is past `stamped.revision`.
    fn wait(&self, stamped: Reading) -> impl Future<Output = ()> + Send;
}
