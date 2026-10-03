//! Fixtures: theories and facts written with readable strings and interned
//! on load, so the core never sees a string.
//!
//! Theory JSON maps each theory to its relations, and each relation to its
//! rewrite in the JSON form described on [`Rewrite`]:
//!
//! ```json
//! {
//!   "file": {
//!     "owner": "this",
//!     "editor": { "union": ["this", { "computed": "owner" }] }
//!   }
//! }
//! ```
//!
//! Fact JSON is a list of objects, each with a `set` and one subject key:
//!
//! ```json
//! [
//!   { "set": "file:readme#owner", "identity": "alice" },
//!   { "set": "file:readme#parent", "resource": "folder:root" },
//!   { "set": "folder:root#viewer", "subjectset": "group:eng#member" }
//! ]
//! ```
//!
//! Resource ids are opaque, but the text still splits unambiguously:
//! theory names contain no `:` and relation names contain no `#`.

use std::collections::BTreeMap;

use serde::Deserialize;
use thiserror::Error;

use crate::{
    memory::{MemoryDictionary, MemoryFactStore},
    model::{Fact, IdentityId, RelationId, Resource, ResourceId, Subject, Subjectset, TheoryId},
    rewrite::Rewrite,
    theory::{Theories, TheoryError},
};

#[derive(Debug, Error)]
pub enum FixtureError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Theory(#[from] TheoryError),
    #[error("malformed resource {0:?}, expected theory:id")]
    Resource(String),
    #[error("malformed subjectset {0:?}, expected theory:id#relation")]
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

/// Declares `relation` in `theory` with a rewrite that names its relations.
pub fn declare(
    theories: &mut Theories,
    dictionary: &mut MemoryDictionary,
    theory: &str,
    relation: &str,
    rewrite: Rewrite<&str>,
) -> Result<(), TheoryError> {
    declare_named(theories, dictionary, theory, relation, rewrite)
}

/// [`declare`] for any owned or borrowed relation names. A separate
/// function so `declare(.., This)` still infers `&str`.
fn declare_named(
    theories: &mut Theories,
    dictionary: &mut MemoryDictionary,
    theory: &str,
    relation: &str,
    rewrite: Rewrite<impl AsRef<str>>,
) -> Result<(), TheoryError> {
    let theory = TheoryId(dictionary.intern(theory));
    let relation = RelationId(dictionary.intern(relation));
    let rewrite = rewrite.map(&mut |name| RelationId(dictionary.intern(name.as_ref())));
    theories
        .entry(theory)
        .or_default()
        .declare(relation, rewrite)
}

/// The JSON is read in name order, so ids are interned in a fixed order.
pub fn load_theories(
    json: &str,
    dictionary: &mut MemoryDictionary,
    theories: &mut Theories,
) -> Result<(), FixtureError> {
    let parsed: BTreeMap<String, BTreeMap<String, Rewrite<String>>> = serde_json::from_str(json)?;
    for (theory, relations) in parsed {
        for (relation, rewrite) in relations {
            declare_named(theories, dictionary, &theory, &relation, rewrite)?;
        }
    }
    Ok(())
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
    let (theory, id) = text
        .split_once(':')
        .ok_or_else(|| FixtureError::Resource(text.to_owned()))?;
    Ok(Resource {
        theory: TheoryId(dictionary.intern(theory)),
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
