//! The fact graph: facts are edges from a subjectset to a subject.
//! Namespaces and relations are interned ids; resource ids and identities
//! stay names, since only namespace and relation names are interned.

use crate::{
    id::{NamespaceId, RelationId},
    name::{Identity, ResourceId},
};

/// `namespace:id`
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Resource {
    pub namespace: NamespaceId,
    pub id: ResourceId,
}

/// `namespace:id#relation`
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Subjectset {
    pub resource: Resource,
    pub relation: RelationId,
}

/// The right-hand side of a fact.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Subject {
    Identity(Identity),
    Subjectset(Subjectset),
    /// `namespace:id#...`, the resource itself.
    ResourceMember(Resource),
}

/// `subjectset@subject`
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Fact {
    pub subjectset: Subjectset,
    pub subject: Subject,
}
