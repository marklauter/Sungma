//! Fact documents: the JSON form of a list of facts, as
//! `docs/specs/fact-documents.md` specifies.
//!
//! ```json
//! [
//!   { "namespace": "file", "resource": "readme", "relation": "owner",
//!     "subject": { "identity": "alice" } },
//!   { "namespace": "folder", "resource": "root", "relation": "viewer",
//!     "subject": { "namespace": "group", "resource": "eng", "relation": "member" } },
//!   { "namespace": "file", "resource": "readme", "relation": "parent",
//!     "subject": { "namespace": "folder", "resource": "root" } }
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
    graph::{Fact, Resource, Subject, Subjectset},
    id::{NamespaceId, RelationId},
    name::{
        Identity, NameError, NamespaceName, RelationName, ResourceId, ResourceName, SubjectsetName,
    },
    store::Pool,
};
use sungma_memory::{MemoryDictionary, MemoryFactStore};
use sungma_write::store::InternError;

/// Why a fact document wasn't loaded.
#[derive(Debug, Error)]
pub enum FactError {
    /// The document was refused, with its line and column.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Intern(#[from] InternError),
}

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

const FACT_KEYS: &[&str] = &["namespace", "resource", "relation", "subject"];

const SUBJECT_KEYS: &[&str] = &["identity", "namespace", "resource", "relation"];

const SUBJECT_SHAPES: &str =
    "an identity, a namespace and resource, or a namespace, resource and relation";

/// The keys of a fact or a subject, each read at most once.
#[derive(Default)]
struct Keys {
    identity: Option<Identity>,
    namespace: Option<NamespaceName>,
    resource: Option<ResourceId>,
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
                "namespace" => keys.namespace.is_some(),
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
                "namespace" => keys.namespace = Some(named(map.next_value()?)?),
                "resource" => keys.resource = Some(named(map.next_value()?)?),
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

struct FactVisitor;

impl<'de> Visitor<'de> for FactVisitor {
    type Value = FactDto;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a fact: a namespace, resource, relation and subject")
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
        let keys = Keys::read(map, FACT_KEYS)?;
        let namespace = keys
            .namespace
            .ok_or_else(|| de::Error::missing_field("namespace"))?;
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
            set: SubjectsetName::new(ResourceName::new(namespace, id), relation),
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
        match (keys.identity, keys.namespace, keys.resource, keys.relation) {
            (Some(identity), None, None, None) => Ok(SubjectDto::Identity(identity)),
            (None, Some(namespace), Some(id), None) => {
                Ok(SubjectDto::Resource(ResourceName::new(namespace, id)))
            }
            (None, Some(namespace), Some(id), Some(relation)) => Ok(SubjectDto::Subjectset(
                SubjectsetName::new(ResourceName::new(namespace, id), relation),
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

/// Parses a fact document whole, then interns every fact's namespace and
/// relation names, and then inserts the facts, so a refused document
/// stores no fact.
pub fn load_facts(
    json: &str,
    dictionary: &MemoryDictionary,
    store: &MemoryFactStore,
) -> Result<(), FactError> {
    let dtos = serde_json::from_str::<Vec<FactDto>>(json)?;
    let mut facts = Vec::with_capacity(dtos.len());
    for dto in dtos {
        let subjectset = intern_subjectset(dictionary, &dto.set)?;
        let subject = match dto.subject {
            SubjectDto::Identity(identity) => Subject::Identity(identity),
            SubjectDto::Resource(resource) => {
                Subject::ResourceMember(intern_resource(dictionary, &resource)?)
            }
            SubjectDto::Subjectset(set) => {
                Subject::Subjectset(intern_subjectset(dictionary, &set)?)
            }
        };
        facts.push(Fact {
            subjectset,
            subject,
        });
    }
    for fact in facts {
        store.insert(fact);
    }
    Ok(())
}

fn intern_resource(
    dictionary: &MemoryDictionary,
    name: &ResourceName,
) -> Result<Resource, InternError> {
    Ok(Resource {
        namespace: NamespaceId(dictionary.intern(Pool::Namespaces, name.namespace().as_str())?),
        id: name.id().clone(),
    })
}

fn intern_subjectset(
    dictionary: &MemoryDictionary,
    name: &SubjectsetName,
) -> Result<Subjectset, InternError> {
    Ok(Subjectset {
        resource: intern_resource(dictionary, name.resource())?,
        relation: RelationId(dictionary.intern(Pool::Relations, name.relation().as_str())?),
    })
}
