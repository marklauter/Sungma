//! Fixtures: theories and facts written with readable strings and interned
//! on load, so the core never sees a string.
//!
//! Each theory is a theory document, as [`crate::document`] describes:
//!
//! ```json
//! { "file": { "owner": "this", "editor": "this | owner" } }
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
//! Names are parsed as [`crate::name`] describes.

use serde::Deserialize;
use thiserror::Error;

use std::collections::HashSet;

use crate::{
    document::{self, DocumentError},
    memory::{MemoryDictionary, MemoryFactStore, MemoryTheoryStore},
    model::{Fact, IdentityId, RelationId, Resource, ResourceId, Subject, Subjectset, TheoryId},
    name::{NameError, ResourceName, SubjectsetName},
    rewrite::Rewrite,
    store::Pool,
    theory::{Theory, TheoryError},
};

#[derive(Debug, Error)]
pub enum FixtureError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("{}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
    Document(Vec<DocumentError>),
    #[error("theory '{0}' is declared by more than one document")]
    DuplicateTheory(String),
    #[error(transparent)]
    Name(#[from] NameError),
    #[error("fact '{0}' needs exactly one of identity, resource or subjectset")]
    Subject(String),
}

// Not a flattened enum: flatten keeps the first subject key it finds and
// ignores the rest, so a fact with two subjects would load as the first.
#[derive(Deserialize)]
struct FactDto {
    set: String,
    identity: Option<String>,
    resource: Option<String>,
    subjectset: Option<String>,
}

/// Declares `relation` in `theory` with a rewrite that names its relations,
/// keeping the theory's other relations. The theory is checked whole, and
/// refused with its first problem, which names relations by id.
pub fn declare(
    theories: &mut MemoryTheoryStore,
    dictionary: &MemoryDictionary,
    theory: &str,
    relation: &str,
    rewrite: Rewrite<&str>,
) -> Result<(), TheoryError> {
    let theory = TheoryId(dictionary.intern(Pool::Theories, theory));
    let relation = RelationId(dictionary.intern(Pool::Relations, relation));
    let rewrite = rewrite.map(&mut |name| RelationId(dictionary.intern(Pool::Relations, name)));
    let mut relations: Vec<_> = theories
        .theory(theory)
        .into_iter()
        .flat_map(Theory::relations)
        .filter(|(declared, _)| **declared != relation)
        .map(|(declared, rewrite)| (*declared, rewrite.clone()))
        .collect();
    relations.push((relation, rewrite));
    let checked = Theory::new(relations).map_err(|mut problems| problems.remove(0).error)?;
    theories.declare(theory, checked);
    Ok(())
}

/// Declares the theory each document declares. Two documents declaring
/// one theory are refused, as when a stale copy sits beside the current one.
pub fn load_theories(
    documents: &[&str],
    dictionary: &MemoryDictionary,
    theories: &mut MemoryTheoryStore,
) -> Result<(), FixtureError> {
    let mut seen = HashSet::new();
    for document in documents {
        let document = document::parse(document).map_err(FixtureError::Document)?;
        if !seen.insert(document.theory.clone()) {
            return Err(FixtureError::DuplicateTheory(document.theory.to_string()));
        }
        let id = TheoryId(dictionary.intern(Pool::Theories, document.theory.as_str()));
        let relations = document
            .relations
            .map(|name| RelationId(dictionary.intern(Pool::Relations, name.as_str())));
        theories.declare(id, relations);
    }
    Ok(())
}

pub fn load_facts(
    json: &str,
    dictionary: &MemoryDictionary,
    store: &MemoryFactStore,
) -> Result<(), FixtureError> {
    for dto in serde_json::from_str::<Vec<FactDto>>(json)? {
        let subjectset = intern_subjectset(dictionary, &dto.set)?;
        let subject = match (dto.identity, dto.resource, dto.subjectset) {
            (Some(identity), None, None) => {
                Subject::Identity(IdentityId(dictionary.intern(Pool::Identities, &identity)))
            }
            (None, Some(resource), None) => {
                Subject::ResourceMember(intern_resource(dictionary, &resource)?)
            }
            (None, None, Some(set)) => Subject::Subjectset(intern_subjectset(dictionary, &set)?),
            _ => return Err(FixtureError::Subject(dto.set)),
        };
        store.insert(Fact {
            subjectset,
            subject,
        });
    }
    Ok(())
}

fn intern_resource(dictionary: &MemoryDictionary, text: &str) -> Result<Resource, NameError> {
    let name: ResourceName = text.parse()?;
    Ok(intern_resource_name(dictionary, &name))
}

fn intern_resource_name(dictionary: &MemoryDictionary, name: &ResourceName) -> Resource {
    let theory = TheoryId(dictionary.intern(Pool::Theories, name.theory().as_str()));
    Resource {
        theory,
        id: ResourceId(dictionary.intern(Pool::Resources(theory), name.id())),
    }
}

fn intern_subjectset(dictionary: &MemoryDictionary, text: &str) -> Result<Subjectset, NameError> {
    let name: SubjectsetName = text.parse()?;
    Ok(Subjectset {
        resource: intern_resource_name(dictionary, name.resource()),
        relation: RelationId(dictionary.intern(Pool::Relations, name.relation().as_str())),
    })
}
