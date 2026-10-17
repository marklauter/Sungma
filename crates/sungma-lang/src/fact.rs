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
//! as [`sungma::name`] describes. [`parse`] gives the facts as named, and
//! touches no store; a write resolves the names. Namespace and relation
//! names are looked up, never interned: a name is interned when a
//! namespace is written, so a fact can only name what a namespace declared.

use std::{fmt, str::FromStr};

use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, Visitor},
};
use serde_json::{Map, Value, json};

use sungma::name::{
    FactName, Identity, NameError, NamespaceName, RelationName, ResourceId, ResourceName,
    SubjectName, SubjectsetName,
};

/// One fact as written.
struct FactDto(FactName);

/// One subject as written.
struct SubjectDto(SubjectName);

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
        Ok(FactDto(FactName {
            subjectset: SubjectsetName::new(ResourceName::new(namespace, id), relation),
            subject: subject.0,
        }))
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
            (Some(identity), None, None, None) => Ok(SubjectDto(SubjectName::Identity(identity))),
            (None, Some(namespace), Some(id), None) => Ok(SubjectDto(SubjectName::Resource(
                ResourceName::new(namespace, id),
            ))),
            (None, Some(namespace), Some(id), Some(relation)) => {
                Ok(SubjectDto(SubjectName::Subjectset(SubjectsetName::new(
                    ResourceName::new(namespace, id),
                    relation,
                ))))
            }
            _ => Err(de::Error::custom(format!("a subject is {SUBJECT_SHAPES}"))),
        }
    }
}

impl<'de> Deserialize<'de> for SubjectDto {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(SubjectVisitor)
    }
}

/// Reads a subject in a fact document's form, for a document that holds
/// one, such as a read request. Use it with `#[serde(deserialize_with)]`.
pub fn subject<'de, D: Deserializer<'de>>(deserializer: D) -> Result<SubjectName, D::Error> {
    SubjectDto::deserialize(deserializer).map(|subject| subject.0)
}

/// Parses a fact document whole. A refused document gives no facts.
pub fn parse(json: &str) -> Result<Vec<FactName>, serde_json::Error> {
    let dtos = serde_json::from_str::<Vec<FactDto>>(json)?;
    Ok(dtos.into_iter().map(|dto| dto.0).collect())
}

/// A write request, `{ "add": [..], "retire": [..] }`, each list a fact
/// document. Both keys are optional, but add and retire can't both be
/// empty: a request with no facts is refused.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
#[serde(try_from = "WriteDto")]
pub struct WriteRequest {
    pub add: Vec<FactName>,
    pub retire: Vec<FactName>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteDto {
    add: Option<Vec<FactDto>>,
    retire: Option<Vec<FactDto>>,
}

impl TryFrom<WriteDto> for WriteRequest {
    type Error = &'static str;

    fn try_from(dto: WriteDto) -> Result<Self, Self::Error> {
        let facts =
            |dtos: Option<Vec<FactDto>>| dtos.into_iter().flatten().map(|dto| dto.0).collect();
        let request = Self {
            add: facts(dto.add),
            retire: facts(dto.retire),
        };
        if request.add.is_empty() && request.retire.is_empty() {
            return Err("a write request adds or retires at least one fact");
        }
        Ok(request)
    }
}

/// Parses a [`WriteRequest`].
pub fn parse_write(json: &str) -> Result<WriteRequest, serde_json::Error> {
    serde_json::from_str(json)
}

/// The facts as a fact document.
pub fn document(facts: &[FactName]) -> Value {
    Value::Array(facts.iter().map(fact_value).collect())
}

fn fact_value(fact: &FactName) -> Value {
    let mut value = subjectset_value(&fact.subjectset);
    let subject = match &fact.subject {
        SubjectName::Identity(identity) => json!({ "identity": identity.as_str() }),
        SubjectName::Resource(resource) => Value::Object(resource_value(resource)),
        SubjectName::Subjectset(set) => Value::Object(subjectset_value(set)),
    };
    value.insert("subject".to_owned(), subject);
    Value::Object(value)
}

fn resource_value(resource: &ResourceName) -> Map<String, Value> {
    let mut value = Map::new();
    value.insert("namespace".to_owned(), resource.namespace().as_str().into());
    value.insert("resource".to_owned(), resource.id().as_str().into());
    value
}

fn subjectset_value(set: &SubjectsetName) -> Map<String, Value> {
    let mut value = resource_value(set.resource());
    value.insert("relation".to_owned(), set.relation().as_str().into());
    value
}
