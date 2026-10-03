//! Resolves the strings of a request to interned ids. A name that was
//! never interned appears in no fact, so its check is denied without
//! touching the fact store.

use crate::{
    model::{IdentityId, RelationId, Resource, ResourceId, Subjectset, TheoryId},
    store::{Dictionary, StoreError},
};

pub async fn resource<D: Dictionary>(
    dictionary: &D,
    theory: &str,
    id: &str,
) -> Result<Option<Resource>, StoreError> {
    let (Some(theory), Some(id)) = (
        dictionary.lookup(theory).await?,
        dictionary.lookup(id).await?,
    ) else {
        return Ok(None);
    };
    Ok(Some(Resource {
        theory: TheoryId(theory),
        id: ResourceId(id),
    }))
}

pub async fn subjectset<D: Dictionary>(
    dictionary: &D,
    theory: &str,
    id: &str,
    relation: &str,
) -> Result<Option<Subjectset>, StoreError> {
    let (Some(resource), Some(relation)) = (
        resource(dictionary, theory, id).await?,
        dictionary.lookup(relation).await?,
    ) else {
        return Ok(None);
    };
    Ok(Some(Subjectset {
        resource,
        relation: RelationId(relation),
    }))
}

pub async fn identity<D: Dictionary>(
    dictionary: &D,
    identity: &str,
) -> Result<Option<IdentityId>, StoreError> {
    Ok(dictionary.lookup(identity).await?.map(IdentityId))
}
