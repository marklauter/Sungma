//! Facts and their parts, over interned integer ids.

/// An interned namespace path, `theory/namespace`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NamespaceId(pub u32);

/// An interned relation name.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct RelationId(pub u32);

/// An interned resource id. The string behind it is opaque to Sungma.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ResourceId(pub u32);

/// An interned identity. The string behind it is opaque to Sungma.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct IdentityId(pub u32);

/// `namespace:id`
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Resource {
    pub namespace: NamespaceId,
    pub id: ResourceId,
}

/// `namespace:id#relation`
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Subjectset {
    pub resource: Resource,
    pub relation: RelationId,
}

/// The right-hand side of a fact.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Subject {
    Identity(IdentityId),
    Subjectset(Subjectset),
    /// `namespace:id#...`, the resource itself.
    ResourceMember(Resource),
}

/// `subjectset@subject`
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Fact {
    pub subjectset: Subjectset,
    pub subject: Subject,
}
