//! The revision clock.

/// Each write produces the next revision, and a read at a revision sees
/// every fact written at or before it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Revision(pub u64);
