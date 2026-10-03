//! Resolves the strings of a request to interned ids. A name that was
//! never interned appears in no fact, so its check is denied without
//! touching the fact store.

use crate::{
    model::{IdentityId, NamespaceId, RelationId, Resource, ResourceId, Subjectset},
    store::{Dictionary, StoreError},
};

pub async fn subjectset<D: Dictionary>(
    dictionary: &D,
    namespace: &str,
    id: &str,
    relation: &str,
) -> Result<Option<Subjectset>, StoreError> {
    let (Some(namespace), Some(id), Some(relation)) = (
        dictionary.lookup(namespace).await?,
        dictionary.lookup(id).await?,
        dictionary.lookup(relation).await?,
    ) else {
        return Ok(None);
    };
    Ok(Some(Subjectset {
        resource: Resource {
            namespace: NamespaceId(namespace),
            id: ResourceId(id),
        },
        relation: RelationId(relation),
    }))
}

pub async fn identity<D: Dictionary>(
    dictionary: &D,
    identity: &str,
) -> Result<Option<IdentityId>, StoreError> {
    Ok(dictionary.lookup(identity).await?.map(IdentityId))
}
