//! Resolves the strings of a request to interned ids. A name that was
//! never interned appears in no fact, so its check is denied without
//! touching the fact store.

use crate::{
    model::{IdentityId, RelationId, Resource, ResourceId, Subjectset, TheoryId},
    store::{Dictionary, StoreError},
};

pub async fn subjectset<D: Dictionary>(
    dictionary: &D,
    theory: &str,
    id: &str,
    relation: &str,
) -> Result<Option<Subjectset>, StoreError> {
    let (Some(theory), Some(id), Some(relation)) = (
        dictionary.lookup(theory).await?,
        dictionary.lookup(id).await?,
        dictionary.lookup(relation).await?,
    ) else {
        return Ok(None);
    };
    Ok(Some(Subjectset {
        resource: Resource {
            theory: TheoryId(theory),
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
