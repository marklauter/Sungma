//! Interned integer ids, which stand for the names in [`crate::name`].

use std::fmt;

/// An interned theory name.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct TheoryId(pub u32);

/// An interned relation name.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct RelationId(pub u32);

/// The bare id, for messages about relations whose names aren't at hand.
impl fmt::Display for RelationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// An interned resource id. The string behind it is opaque to Sungma.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ResourceId(pub u32);

/// An interned identity. The string behind it is opaque to Sungma.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct IdentityId(pub u32);
