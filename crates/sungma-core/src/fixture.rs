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
//! Names are parsed as [`crate::name`] describes.

use std::{collections::HashSet, fmt, marker::PhantomData};

use serde::{
    Deserialize, Deserializer,
    de::{MapAccess, Visitor},
};
use thiserror::Error;

use crate::{
    memory::{MemoryDictionary, MemoryFactStore, MemoryTheoryStore},
    model::{Fact, IdentityId, RelationId, Resource, ResourceId, Subject, Subjectset, TheoryId},
    name::{NameError, ResourceName, SubjectsetName},
    rewrite::Rewrite,
    theory::{self, Theory, TheoryError},
};

#[derive(Debug, Error)]
pub enum FixtureError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Theory(#[from] TheoryError),
    #[error("theory '{0}' is declared more than once")]
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
/// its errors name relations by id.
pub fn declare(
    theories: &mut MemoryTheoryStore,
    dictionary: &mut MemoryDictionary,
    theory: &str,
    relation: &str,
    rewrite: Rewrite<&str>,
) -> Result<(), TheoryError> {
    let theory = TheoryId(dictionary.intern(theory));
    let relation = RelationId(dictionary.intern(relation));
    let rewrite = rewrite.map(&mut |name| RelationId(dictionary.intern(name)));
    let mut relations: Vec<_> = theories
        .theory(theory)
        .into_iter()
        .flat_map(Theory::relations)
        .filter(|(declared, _)| *declared != relation)
        .map(|(declared, rewrite)| (declared, rewrite.clone()))
        .collect();
    relations.push((relation, rewrite));
    theories.declare(theory, Theory::new(relations)?);
    Ok(())
}

/// Declares each theory whole, replacing any earlier declaration. Each is
/// checked by name before its names are interned, in document order.
pub fn load_theories(
    json: &str,
    dictionary: &mut MemoryDictionary,
    theories: &mut MemoryTheoryStore,
) -> Result<(), FixtureError> {
    let Entries(parsed) = serde_json::from_str::<Entries<Entries<Rewrite<String>>>>(json)?;
    let mut seen = HashSet::new();
    for (theory, Entries(relations)) in parsed {
        if !seen.insert(theory.clone()) {
            return Err(FixtureError::DuplicateTheory(theory));
        }
        theory::validate(&relations)?;
        let id = TheoryId(dictionary.intern(&theory));
        let relations = relations
            .into_iter()
            .map(|(relation, rewrite)| {
                let relation = RelationId(dictionary.intern(&relation));
                (
                    relation,
                    rewrite.map(&mut |name| RelationId(dictionary.intern(&name))),
                )
            })
            .collect();
        theories.declare(id, Theory::new(relations)?);
    }
    Ok(())
}

/// A JSON object read as its entries in document order, duplicates kept,
/// so validation sees a duplicate a map would hide.
struct Entries<V>(Vec<(String, V)>);

impl<'de, V: Deserialize<'de>> Deserialize<'de> for Entries<V> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visit<V>(PhantomData<V>);

        impl<'de, V: Deserialize<'de>> Visitor<'de> for Visit<V> {
            type Value = Entries<V>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry()? {
                    entries.push(entry);
                }
                Ok(Entries(entries))
            }
        }

        deserializer.deserialize_map(Visit(PhantomData))
    }
}

pub fn load_facts(
    json: &str,
    dictionary: &mut MemoryDictionary,
    store: &mut MemoryFactStore,
) -> Result<(), FixtureError> {
    for dto in serde_json::from_str::<Vec<FactDto>>(json)? {
        let subjectset = intern_subjectset(dictionary, &dto.set)?;
        let subject = match (dto.identity, dto.resource, dto.subjectset) {
            (Some(identity), None, None) => {
                Subject::Identity(IdentityId(dictionary.intern(&identity)))
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

fn intern_resource(dictionary: &mut MemoryDictionary, text: &str) -> Result<Resource, NameError> {
    let name: ResourceName = text.parse()?;
    Ok(intern_resource_name(dictionary, &name))
}

fn intern_resource_name(dictionary: &mut MemoryDictionary, name: &ResourceName) -> Resource {
    Resource {
        theory: TheoryId(dictionary.intern(name.theory())),
        id: ResourceId(dictionary.intern(name.id())),
    }
}

fn intern_subjectset(
    dictionary: &mut MemoryDictionary,
    text: &str,
) -> Result<Subjectset, NameError> {
    let name: SubjectsetName = text.parse()?;
    Ok(Subjectset {
        resource: intern_resource_name(dictionary, name.resource()),
        relation: RelationId(dictionary.intern(name.relation())),
    })
}
