//! Resolves names to the graph's values through a [`Dictionary`], and back.
//! Only namespace and relation names are interned; resource ids and
//! identities are kept as names. A name never interned appears in no fact,
//! so a read that meets one finds nothing, and a write that meets one is
//! refused.

use thiserror::Error;

use sungma::{
    graph::{Fact, Resource, Subject, Subjectset},
    id::{NamespaceId, RelationId},
    name::{FactName, NamespaceName, RelationName, ResourceName, SubjectName, SubjectsetName},
    namespace::Namespace,
    store::StoreError,
};

use crate::store::{Dictionary, Pool};

/// A namespace or relation name that was never interned.
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum UnknownName {
    #[error("namespace '{0}' was never declared")]
    Namespace(NamespaceName),
    #[error("relation '{0}' was never declared")]
    Relation(RelationName),
}

#[derive(Debug, Error)]
pub enum ResolveError {
    #[error(transparent)]
    Unknown(#[from] UnknownName),
    #[error(transparent)]
    Store(#[from] StoreError),
}

pub async fn namespace<D: Dictionary>(
    dictionary: &D,
    name: &NamespaceName,
) -> Result<NamespaceId, ResolveError> {
    let id = dictionary.lookup(Pool::Namespaces, name.as_str()).await?;
    Ok(NamespaceId(
        id.ok_or_else(|| UnknownName::Namespace(name.clone()))?,
    ))
}

pub async fn relation<D: Dictionary>(
    dictionary: &D,
    name: &RelationName,
) -> Result<RelationId, ResolveError> {
    let id = dictionary.lookup(Pool::Relations, name.as_str()).await?;
    Ok(RelationId(
        id.ok_or_else(|| UnknownName::Relation(name.clone()))?,
    ))
}

pub async fn resource<D: Dictionary>(
    dictionary: &D,
    name: &ResourceName,
) -> Result<Resource, ResolveError> {
    Ok(Resource {
        namespace: namespace(dictionary, name.namespace()).await?,
        id: name.id().clone(),
    })
}

/// Looks the relation up first, then the namespace.
pub async fn subjectset<D: Dictionary>(
    dictionary: &D,
    name: &SubjectsetName,
) -> Result<Subjectset, ResolveError> {
    let relation = relation(dictionary, name.relation()).await?;
    Ok(Subjectset {
        resource: resource(dictionary, name.resource()).await?,
        relation,
    })
}

pub async fn subject<D: Dictionary>(
    dictionary: &D,
    name: &SubjectName,
) -> Result<Subject, ResolveError> {
    Ok(match name {
        SubjectName::Identity(identity) => Subject::Identity(identity.clone()),
        SubjectName::Resource(resource) => {
            Subject::ResourceMember(self::resource(dictionary, resource).await?)
        }
        SubjectName::Subjectset(set) => Subject::Subjectset(subjectset(dictionary, set).await?),
    })
}

/// The subjectset first, then the subject.
pub async fn fact<D: Dictionary>(dictionary: &D, name: &FactName) -> Result<Fact, ResolveError> {
    Ok(Fact {
        subjectset: subjectset(dictionary, &name.subjectset).await?,
        subject: subject(dictionary, &name.subject).await?,
    })
}

/// A stored id with no name, or a stored name the grammar refuses, is a
/// store failure: interning stores only names the grammar accepts.
async fn named<D: Dictionary, N: std::str::FromStr>(
    dictionary: &D,
    pool: Pool,
    id: u32,
) -> Result<N, StoreError> {
    let name = dictionary
        .name(pool, id)
        .await?
        .ok_or_else(|| StoreError(format!("{pool:?} id {id} has no name")))?;
    name.parse().map_err(|_| {
        StoreError(format!(
            "{pool:?} id {id} names '{name}', which isn't a name"
        ))
    })
}

pub async fn namespace_name<D: Dictionary>(
    dictionary: &D,
    id: NamespaceId,
) -> Result<NamespaceName, StoreError> {
    named(dictionary, Pool::Namespaces, id.0).await
}

pub async fn relation_name<D: Dictionary>(
    dictionary: &D,
    id: RelationId,
) -> Result<RelationName, StoreError> {
    named(dictionary, Pool::Relations, id.0).await
}

pub async fn resource_name<D: Dictionary>(
    dictionary: &D,
    resource: &Resource,
) -> Result<ResourceName, StoreError> {
    Ok(ResourceName::new(
        namespace_name(dictionary, resource.namespace).await?,
        resource.id.clone(),
    ))
}

pub async fn subjectset_name<D: Dictionary>(
    dictionary: &D,
    set: &Subjectset,
) -> Result<SubjectsetName, StoreError> {
    Ok(SubjectsetName::new(
        resource_name(dictionary, &set.resource).await?,
        relation_name(dictionary, set.relation).await?,
    ))
}

pub async fn subject_name<D: Dictionary>(
    dictionary: &D,
    subject: &Subject,
) -> Result<SubjectName, StoreError> {
    Ok(match subject {
        Subject::Identity(identity) => SubjectName::Identity(identity.clone()),
        Subject::ResourceMember(resource) => {
            SubjectName::Resource(resource_name(dictionary, resource).await?)
        }
        Subject::Subjectset(set) => {
            SubjectName::Subjectset(subjectset_name(dictionary, set).await?)
        }
    })
}

/// The namespace with every relation id replaced by its name.
pub async fn namespace_names<D: Dictionary>(
    dictionary: &D,
    namespace: &Namespace,
) -> Result<Namespace<RelationName>, StoreError> {
    let mut names = std::collections::HashMap::new();
    for id in namespace.names() {
        names.insert(*id, relation_name(dictionary, *id).await?);
    }
    // Every id the namespace names was just named, and distinct ids have
    // distinct names in one pool, so the rename can't fail; a store that
    // broke that is reported as failed.
    namespace
        .try_map(|id| {
            names
                .get(id)
                .cloned()
                .ok_or_else(|| Renamed(format!("relation id {id} has no name")))
        })
        .map_err(|Renamed(error)| StoreError(error))
}

/// A rename that failed, as a store failure.
struct Renamed(String);

impl From<sungma::namespace::NamespaceError> for Renamed {
    fn from(error: sungma::namespace::NamespaceError) -> Self {
        Self(error.to_string())
    }
}

/// A resolution that met a name never interned as `None`, as a read
/// treats one: no fact mentions it.
pub fn known<T>(resolved: Result<T, ResolveError>) -> Result<Option<T>, StoreError> {
    match resolved {
        Ok(value) => Ok(Some(value)),
        Err(ResolveError::Unknown(_)) => Ok(None),
        Err(ResolveError::Store(error)) => Err(error),
    }
}
