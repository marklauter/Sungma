//! Test fixtures: theories and facts written with readable strings and
//! interned on load, so the core never sees a string.
//!
//! Fact JSON is a list of objects, each with a `set` and one subject key:
//!
//! ```json
//! [
//!   { "set": "docs/file:readme#owner", "identity": "alice" },
//!   { "set": "docs/file:readme#parent", "resource": "docs/folder:root" },
//!   { "set": "docs/folder:root#viewer", "subjectset": "docs/group:eng#member" }
//! ]
//! ```
//!
//! Resource ids are opaque, but the text still splits unambiguously:
//! namespace paths contain no `:` and relation names contain no `#`.

use serde::Deserialize;
use thiserror::Error;

use crate::{
    memory::{MemoryDictionary, MemoryFactStore},
    model::{Fact, IdentityId, NamespaceId, RelationId, Resource, ResourceId, Subject, Subjectset},
    rewrite::Rewrite,
    theory::{Theory, TheoryError},
};

#[derive(Debug, Error)]
pub enum FixtureError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("malformed resource {0:?}, expected namespace:id")]
    Resource(String),
    #[error("malformed subjectset {0:?}, expected namespace:id#relation")]
    Subjectset(String),
}

#[derive(Deserialize)]
struct FactDto {
    set: String,
    #[serde(flatten)]
    subject: SubjectDto,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum SubjectDto {
    Identity(String),
    Resource(String),
    Subjectset(String),
}

/// Declares `namespace#relation` with a rewrite that names its relations.
pub fn declare(
    theory: &mut Theory,
    dictionary: &mut MemoryDictionary,
    namespace: &str,
    relation: &str,
    rewrite: Rewrite<&str>,
) -> Result<(), TheoryError> {
    let namespace = NamespaceId(dictionary.intern(namespace));
    let relation = RelationId(dictionary.intern(relation));
    let rewrite = rewrite.map(&mut |name| RelationId(dictionary.intern(name)));
    theory.declare(namespace, relation, rewrite)
}

pub fn load_facts(
    json: &str,
    dictionary: &mut MemoryDictionary,
    store: &mut MemoryFactStore,
) -> Result<(), FixtureError> {
    for dto in serde_json::from_str::<Vec<FactDto>>(json)? {
        let subjectset = intern_subjectset(dictionary, &dto.set)?;
        let subject = match dto.subject {
            SubjectDto::Identity(identity) => {
                Subject::Identity(IdentityId(dictionary.intern(&identity)))
            }
            SubjectDto::Resource(resource) => {
                Subject::ResourceMember(intern_resource(dictionary, &resource)?)
            }
            SubjectDto::Subjectset(set) => {
                Subject::Subjectset(intern_subjectset(dictionary, &set)?)
            }
        };
        store.insert(Fact {
            subjectset,
            subject,
        });
    }
    Ok(())
}

fn intern_resource(
    dictionary: &mut MemoryDictionary,
    text: &str,
) -> Result<Resource, FixtureError> {
    let (namespace, id) = text
        .split_once(':')
        .ok_or_else(|| FixtureError::Resource(text.to_owned()))?;
    Ok(Resource {
        namespace: NamespaceId(dictionary.intern(namespace)),
        id: ResourceId(dictionary.intern(id)),
    })
}

fn intern_subjectset(
    dictionary: &mut MemoryDictionary,
    text: &str,
) -> Result<Subjectset, FixtureError> {
    let (resource, relation) = text
        .rsplit_once('#')
        .ok_or_else(|| FixtureError::Subjectset(text.to_owned()))?;
    Ok(Subjectset {
        resource: intern_resource(dictionary, resource)?,
        relation: RelationId(dictionary.intern(relation)),
    })
}
