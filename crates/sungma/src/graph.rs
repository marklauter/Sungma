//! The fact graph: facts are edges from a subjectset to a subject, over
//! interned ids.

use crate::id::{IdentityId, RelationId, ResourceId, TheoryId};

/// `theory:id`
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Resource {
    pub theory: TheoryId,
    pub id: ResourceId,
}

/// `theory:id#relation`
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Subjectset {
    pub resource: Resource,
    pub relation: RelationId,
}

/// The right-hand side of a fact.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Subject {
    Identity(IdentityId),
    Subjectset(Subjectset),
    /// `theory:id#...`, the resource itself.
    ResourceMember(Resource),
}

/// `subjectset@subject`
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Fact {
    pub subjectset: Subjectset,
    pub subject: Subject,
}
