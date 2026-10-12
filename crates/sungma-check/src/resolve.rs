//! Resolves the names of a request to the graph's values. Only namespace
//! and relation names are interned; a name never interned appears in no
//! fact, so its check is denied without touching the fact store. Resource
//! ids and identities are kept as names.

use sungma::{
    graph::{Resource, Subjectset},
    id::{NamespaceId, RelationId},
    name::{ResourceName, SubjectsetName},
    store::{Dictionary, Pool, StoreError},
};

pub async fn resource<D: Dictionary>(
    dictionary: &D,
    name: &ResourceName,
) -> Result<Option<Resource>, StoreError> {
    Ok(dictionary
        .lookup(Pool::Namespaces, name.namespace().as_str())
        .await?
        .map(|namespace| Resource {
            namespace: NamespaceId(namespace),
            id: name.id().clone(),
        }))
}

pub async fn subjectset<D: Dictionary>(
    dictionary: &D,
    name: &SubjectsetName,
) -> Result<Option<Subjectset>, StoreError> {
    let (Some(resource), Some(relation)) = (
        resource(dictionary, name.resource()).await?,
        dictionary
            .lookup(Pool::Relations, name.relation().as_str())
            .await?,
    ) else {
        return Ok(None);
    };
    Ok(Some(Subjectset {
        resource,
        relation: RelationId(relation),
    }))
}
