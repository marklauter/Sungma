//! Interned integer ids, which stand for the names in [`crate::name`].

use std::fmt;

/// An interned namespace name.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct NamespaceId(pub u32);

/// An interned relation name.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct RelationId(pub u32);

/// The bare id, for messages about relations whose names aren't at hand.
impl fmt::Display for RelationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
