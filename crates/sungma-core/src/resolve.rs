//! Resolves the strings of a request to interned ids. A name that was
//! never interned appears in no fact, so its check is denied without
//! touching the fact store.

use crate::{
    model::{IdentityId, RelationId, Resource, ResourceId, Subjectset, TheoryId},
    name::{ResourceName, SubjectsetName},
    store::{Dictionary, StoreError},
};

pub async fn resource<D: Dictionary>(
    dictionary: &D,
    name: &ResourceName,
) -> Result<Option<Resource>, StoreError> {
    let (Some(theory), Some(id)) = (
        dictionary.lookup(name.theory()).await?,
        dictionary.lookup(name.id()).await?,
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
    name: &SubjectsetName,
) -> Result<Option<Subjectset>, StoreError> {
    let (Some(resource), Some(relation)) = (
        resource(dictionary, name.resource()).await?,
        dictionary.lookup(name.relation()).await?,
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
