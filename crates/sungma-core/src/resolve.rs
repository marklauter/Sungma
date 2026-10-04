//! Resolves the strings of a request to interned ids. A name that was
//! never interned appears in no fact, so its check is denied without
//! touching the fact store.

use crate::{
    model::{IdentityId, RelationId, Resource, ResourceId, Subjectset, TheoryId},
    name::{ResourceName, SubjectsetName},
    store::{Dictionary, Pool, StoreError},
};

pub async fn resource<D: Dictionary>(
    dictionary: &D,
    name: &ResourceName,
) -> Result<Option<Resource>, StoreError> {
    let Some(theory) = dictionary.lookup(Pool::Theories, name.theory()).await? else {
        return Ok(None);
    };
    let theory = TheoryId(theory);
    let id = dictionary
        .lookup(Pool::Resources(theory), name.id())
        .await?;
    Ok(id.map(|id| Resource {
        theory,
        id: ResourceId(id),
    }))
}

pub async fn subjectset<D: Dictionary>(
    dictionary: &D,
    name: &SubjectsetName,
) -> Result<Option<Subjectset>, StoreError> {
    let (Some(resource), Some(relation)) = (
        resource(dictionary, name.resource()).await?,
        dictionary.lookup(Pool::Relations, name.relation()).await?,
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
    Ok(dictionary
        .lookup(Pool::Identities, identity)
        .await?
        .map(IdentityId))
}
