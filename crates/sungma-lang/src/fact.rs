//! Fact documents: the JSON form of a list of facts, as
//! `docs/specs/fact-documents.md` specifies.
//!
//! ```json
//! [
//!   { "theory": "file", "resource": "readme", "relation": "owner",
//!     "subject": { "identity": "alice" } },
//!   { "theory": "folder", "resource": "root", "relation": "viewer",
//!     "subject": { "theory": "group", "resource": "eng", "relation": "member" } },
//!   { "theory": "file", "resource": "readme", "relation": "parent",
//!     "subject": { "theory": "folder", "resource": "root" } }
//! ]
//! ```
//!
//! Every part has its own key, so nothing is split. Every name is checked,
//! as [`sungma::name`] describes, before any is interned, so a refused
//! document declares no facts.

use std::{fmt, str::FromStr};

use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, Visitor},
};
use thiserror::Error;

use sungma::{
    memory::{MemoryDictionary, MemoryFactStore},
    model::{Fact, IdentityId, RelationId, Resource, ResourceId, Subject, Subjectset, TheoryId},
    name::{
        Identity, NameError, RelationName, ResourceName, SubjectsetName, TheoryName,
        check_resource_id,
    },
    store::Pool,
};

/// Why a fact document was refused, with its line and column.
#[derive(Debug, Error)]
#[error(transparent)]
pub struct FactError(#[from] serde_json::Error);

/// One fact as written: its subjectset and its subject.
struct FactDto {
    set: SubjectsetName,
    subject: SubjectDto,
}

enum SubjectDto {
    Identity(Identity),
    Resource(ResourceName),
    Subjectset(SubjectsetName),
}

const FACT_KEYS: &[&str] = &["theory", "resource", "relation", "subject"];

const SUBJECT_KEYS: &[&str] = &["identity", "theory", "resource", "relation"];

const SUBJECT_SHAPES: &str =
    "an identity, a theory and resource, or a theory, resource and relation";

/// The keys of a fact or a subject, each read at most once.
#[derive(Default)]
struct Keys {
    identity: Option<Identity>,
    theory: Option<TheoryName>,
    resource: Option<String>,
    relation: Option<RelationName>,
    subject: Option<SubjectDto>,
}

impl Keys {
    /// Reads every key of `map`, refusing a key outside `allowed` and a key
    /// written twice.
    fn read<'de, A: MapAccess<'de>>(
        mut map: A,
        allowed: &'static [&'static str],
    ) -> Result<Self, A::Error> {
        let mut keys = Self::default();
        while let Some(key) = map.next_key::<String>()? {
            let Some(&key) = allowed.iter().find(|allowed| **allowed == key) else {
                return Err(de::Error::unknown_field(&key, allowed));
            };
            let seen = match key {
                "identity" => keys.identity.is_some(),
                "theory" => keys.theory.is_some(),
                "resource" => keys.resource.is_some(),
                "relation" => keys.relation.is_some(),
                _ => keys.subject.is_some(),
            };
            if seen {
                return Err(de::Error::duplicate_field(key));
            }
            // Each value is checked as it is read, so an error carries the
            // value's line and column.
            match key {
                "identity" => keys.identity = Some(named(map.next_value()?)?),
                "theory" => keys.theory = Some(named(map.next_value()?)?),
                "resource" => {
                    let id: String = map.next_value()?;
                    check_resource_id(&id).map_err(de::Error::custom)?;
                    keys.resource = Some(id);
                }
                "relation" => keys.relation = Some(named(map.next_value()?)?),
                _ => keys.subject = Some(map.next_value()?),
            }
        }
        Ok(keys)
    }
}

/// A name parsed from a document's string, refused with the document's
/// line and column.
fn named<T: FromStr<Err = NameError>, E: de::Error>(text: String) -> Result<T, E> {
    text.parse().map_err(E::custom)
}

fn resource<E: de::Error>(theory: TheoryName, id: &str) -> Result<ResourceName, E> {
    ResourceName::new(theory, id).map_err(E::custom)
}

struct FactVisitor;

impl<'de> Visitor<'de> for FactVisitor {
    type Value = FactDto;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a fact: a theory, resource, relation and subject")
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
        let keys = Keys::read(map, FACT_KEYS)?;
        let theory = keys
            .theory
            .ok_or_else(|| de::Error::missing_field("theory"))?;
        let id = keys
            .resource
            .ok_or_else(|| de::Error::missing_field("resource"))?;
        let relation = keys
            .relation
            .ok_or_else(|| de::Error::missing_field("relation"))?;
        let subject = keys
            .subject
            .ok_or_else(|| de::Error::missing_field("subject"))?;
        Ok(FactDto {
            set: SubjectsetName::new(resource(theory, &id)?, relation),
            subject,
        })
    }
}

impl<'de> Deserialize<'de> for FactDto {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(FactVisitor)
    }
}

struct SubjectVisitor;

impl<'de> Visitor<'de> for SubjectVisitor {
    type Value = SubjectDto;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "a subject: {SUBJECT_SHAPES}")
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
        let keys = Keys::read(map, SUBJECT_KEYS)?;
        match (keys.identity, keys.theory, keys.resource, keys.relation) {
            (Some(identity), None, None, None) => Ok(SubjectDto::Identity(identity)),
            (None, Some(theory), Some(id), None) => {
                Ok(SubjectDto::Resource(resource(theory, &id)?))
            }
            (None, Some(theory), Some(id), Some(relation)) => Ok(SubjectDto::Subjectset(
                SubjectsetName::new(resource(theory, &id)?, relation),
            )),
            _ => Err(de::Error::custom(format!("a subject is {SUBJECT_SHAPES}"))),
        }
    }
}

impl<'de> Deserialize<'de> for SubjectDto {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(SubjectVisitor)
    }
}

/// Parses a fact document whole, then interns and inserts each fact.
pub fn load_facts(
    json: &str,
    dictionary: &MemoryDictionary,
    store: &MemoryFactStore,
) -> Result<(), FactError> {
    for dto in serde_json::from_str::<Vec<FactDto>>(json)? {
        let subjectset = intern_subjectset(dictionary, &dto.set);
        let subject = match dto.subject {
            SubjectDto::Identity(identity) => Subject::Identity(IdentityId(
                dictionary.intern(Pool::Identities, identity.as_str()),
            )),
            SubjectDto::Resource(resource) => {
                Subject::ResourceMember(intern_resource(dictionary, &resource))
            }
            SubjectDto::Subjectset(set) => Subject::Subjectset(intern_subjectset(dictionary, &set)),
        };
        store.insert(Fact {
            subjectset,
            subject,
        });
    }
    Ok(())
}

fn intern_resource(dictionary: &MemoryDictionary, name: &ResourceName) -> Resource {
    let theory = TheoryId(dictionary.intern(Pool::Theories, name.theory().as_str()));
    Resource {
        theory,
        id: ResourceId(dictionary.intern(Pool::Resources(theory), name.id())),
    }
}

fn intern_subjectset(dictionary: &MemoryDictionary, name: &SubjectsetName) -> Subjectset {
    Subjectset {
        resource: intern_resource(dictionary, name.resource()),
        relation: RelationId(dictionary.intern(Pool::Relations, name.relation().as_str())),
    }
}
