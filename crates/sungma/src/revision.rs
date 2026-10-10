//! Revisions, which order writes and name the moments reads are made at.

/// A write is stamped with a revision, and a read at a revision sees every
/// fact written at or before it. Revisions need not be unique: writes
/// stamped alike are concurrent.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Revision(pub u64);
