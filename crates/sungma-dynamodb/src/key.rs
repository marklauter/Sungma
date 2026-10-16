//! Every key in the table, built from bytes that sort in the order reads
//! need: an id is 4 bytes and a revision 8, both big-endian, and a resource
//! id or identity is its bytes and a zero byte, which sorts below every
//! printable character. Each key starts with a tag byte naming what kind of
//! item it is. See `docs/specs/dynamodb-adapter.md`.

use std::fmt::Write;

use sungma::{
    graph::{Resource, Subject, Subjectset},
    id::{NamespaceId, RelationId},
    name::{Identity, ResourceId},
    revision::Revision,
    store::{Pool, StoreError},
};

/// The partition tags.
const FACT: u8 = 1;
const NAMESPACE: u8 = 2;
const NAMES: u8 = 3;
const HEAD: u8 = 4;

/// The subject tags, inside a fact's partition.
const IDENTITY: u8 = 1;
const SUBJECTSET: u8 = 2;
const RESOURCE_MEMBER: u8 = 3;

/// The sort key tags inside a namespace's partition.
const LATEST: u8 = 0;
const ENTRY: u8 = 1;

/// The sort key tags inside a name pool's partition.
const COUNTER: u8 = 0;
const NAME: u8 = 1;
const ID: u8 = 2;

fn id(key: &mut Vec<u8>, id: u32) {
    key.extend_from_slice(&id.to_be_bytes());
}

fn revision(key: &mut Vec<u8>, revision: Revision) {
    key.extend_from_slice(&revision.0.to_be_bytes());
}

fn text(key: &mut Vec<u8>, text: &str) {
    key.extend_from_slice(text.as_bytes());
    key.push(0);
}

/// A fact's partition: its subjectset.
pub fn fact(set: &Subjectset) -> Vec<u8> {
    let mut key = vec![FACT];
    id(&mut key, set.resource.namespace.0);
    id(&mut key, set.relation.0);
    text(&mut key, set.resource.id.as_str());
    key
}

/// The subject, which sorts as [`Subject`] does. A fact's pointer has
/// this sort key.
pub fn subject(subject: &Subject) -> Vec<u8> {
    let mut key = Vec::new();
    match subject {
        Subject::Identity(identity) => {
            key.push(IDENTITY);
            text(&mut key, identity.as_str());
        }
        Subject::Subjectset(set) => {
            key.push(SUBJECTSET);
            id(&mut key, set.resource.namespace.0);
            text(&mut key, set.resource.id.as_str());
            id(&mut key, set.relation.0);
        }
        Subject::ResourceMember(resource) => {
            key.push(RESOURCE_MEMBER);
            id(&mut key, resource.namespace.0);
            text(&mut key, resource.id.as_str());
        }
    }
    key
}

/// The start of every subjectset subject's sort key.
pub fn subjectsets() -> Vec<u8> {
    vec![SUBJECTSET]
}

/// A span of the fact of `subject`, written at `written`.
pub fn span(subject_key: &[u8], written: Revision) -> Vec<u8> {
    let mut key = subject_key.to_vec();
    revision(&mut key, written);
    key
}

/// A namespace's partition.
pub fn namespace(namespace: NamespaceId) -> Vec<u8> {
    let mut key = vec![NAMESPACE];
    id(&mut key, namespace.0);
    key
}

/// The namespace's latest item.
pub fn latest() -> Vec<u8> {
    vec![LATEST]
}

/// The namespace's entry that opens at `opens`.
pub fn entry(opens: Revision) -> Vec<u8> {
    let mut key = vec![ENTRY];
    revision(&mut key, opens);
    key
}

/// A name pool's partition.
pub fn names(pool: Pool) -> Vec<u8> {
    vec![
        NAMES,
        match pool {
            Pool::Namespaces => 0,
            Pool::Relations => 1,
        },
    ]
}

/// The pool's id counter.
pub fn counter() -> Vec<u8> {
    vec![COUNTER]
}

/// The item that maps `name` to its id.
pub fn name(name: &str) -> Vec<u8> {
    let mut key = vec![NAME];
    key.extend_from_slice(name.as_bytes());
    key
}

/// The item that maps `id` to its name.
pub fn name_id(name_id: u32) -> Vec<u8> {
    let mut key = vec![ID];
    id(&mut key, name_id);
    key
}

/// The head's partition and sort key.
pub fn head() -> (Vec<u8>, Vec<u8>) {
    (vec![HEAD], vec![0])
}

/// Reads a key back, field by field.
struct Fields<'a>(&'a [u8]);

impl<'a> Fields<'a> {
    fn byte(&mut self) -> Option<u8> {
        let (&first, rest) = self.0.split_first()?;
        self.0 = rest;
        Some(first)
    }

    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let (bytes, rest) = self.0.split_first_chunk::<N>()?;
        self.0 = rest;
        Some(*bytes)
    }

    fn id(&mut self) -> Option<u32> {
        self.take().map(u32::from_be_bytes)
    }

    fn revision(&mut self) -> Option<Revision> {
        self.take().map(|bytes| Revision(u64::from_be_bytes(bytes)))
    }

    fn text(&mut self) -> Option<&'a str> {
        let end = self.0.iter().position(|&byte| byte == 0)?;
        let text = std::str::from_utf8(&self.0[..end]).ok()?;
        self.0 = &self.0[end + 1..];
        Some(text)
    }

    fn resource(&mut self) -> Option<Resource> {
        let namespace = NamespaceId(self.id()?);
        let id = ResourceId::new_unchecked(self.text()?);
        Some(Resource { namespace, id })
    }

    fn subject(&mut self) -> Option<Subject> {
        Some(match self.byte()? {
            IDENTITY => Subject::Identity(Identity::new_unchecked(self.text()?)),
            SUBJECTSET => {
                let resource = self.resource()?;
                let relation = RelationId(self.id()?);
                Subject::Subjectset(Subjectset { resource, relation })
            }
            RESOURCE_MEMBER => Subject::ResourceMember(self.resource()?),
            _ => return None,
        })
    }

    /// `Some` only when every byte was read.
    fn end<T>(&self, value: T) -> Option<T> {
        self.0.is_empty().then_some(value)
    }
}

/// The subject and the revision that wrote the span a span's key names.
pub fn parse_span(pk: &[u8], sk: &[u8]) -> Result<(Subject, Revision), StoreError> {
    let mut fields = Fields(sk);
    fields
        .subject()
        .and_then(|subject| Some((subject, fields.revision()?)))
        .and_then(|span| fields.end(span))
        .ok_or_else(|| malformed(pk, sk))
}

/// The refusal for an item whose key Sungma didn't write.
pub fn malformed(pk: &[u8], sk: &[u8]) -> StoreError {
    StoreError(format!("malformed key {}", describe(pk, sk)))
}

/// The key as text, for logs and test failures, such as
/// `fact 0:readme#2 identity alice @42`. Bytes it can't read print as hex.
pub fn describe(pk: &[u8], sk: &[u8]) -> String {
    let mut text = String::new();
    let mut fields = Fields(pk);
    let partition = match fields.byte() {
        Some(FACT) => (|| {
            let namespace = fields.id()?;
            let relation = fields.id()?;
            let resource = fields.text()?;
            fields.end(format!("fact {namespace}:{resource}#{relation}"))
        })(),
        Some(NAMESPACE) => fields
            .id()
            .and_then(|id| fields.end(format!("namespace {id}"))),
        Some(NAMES) => fields
            .byte()
            .and_then(|pool| fields.end(format!("names {pool}"))),
        Some(HEAD) => fields.end("head".to_owned()),
        _ => None,
    };
    text.push_str(&partition.unwrap_or_else(|| hex(pk)));
    if sk.is_empty() {
        return text;
    }
    let mut fields = Fields(sk);
    let sort = match pk.first() {
        Some(&FACT) => fields.subject().map(|subject| {
            let subject = match subject {
                Subject::Identity(identity) => format!("identity {}", identity.as_str()),
                Subject::Subjectset(set) => format!(
                    "subjectset {}:{}#{}",
                    set.resource.namespace.0,
                    set.resource.id.as_str(),
                    set.relation.0
                ),
                Subject::ResourceMember(resource) => {
                    format!("member {}:{}", resource.namespace.0, resource.id.as_str())
                }
            };
            match fields.revision() {
                Some(written) => format!("{subject} @{}", written.0),
                None => format!("{subject} pointer"),
            }
        }),
        Some(&NAMESPACE) => match fields.byte() {
            Some(LATEST) => Some("latest".to_owned()),
            Some(ENTRY) => fields.revision().map(|opens| format!("@{}", opens.0)),
            _ => None,
        },
        _ => None,
    };
    let _ = write!(
        text,
        " {}",
        sort.and_then(|sort| fields.end(sort))
            .unwrap_or_else(|| hex(sk))
    );
    text
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::from("0x"), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(namespace: u32, id: &str, relation: u32) -> Subjectset {
        Subjectset {
            resource: Resource {
                namespace: NamespaceId(namespace),
                id: ResourceId::new_unchecked(id),
            },
            relation: RelationId(relation),
        }
    }

    fn subjects() -> Vec<Subject> {
        let mut subjects = vec![
            Subject::Identity(Identity::new_unchecked("al")),
            Subject::Identity(Identity::new_unchecked("alice")),
            Subject::Identity(Identity::new_unchecked("bob")),
            Subject::Subjectset(set(0, "a", 9)),
            Subject::Subjectset(set(0, "ab", 1)),
            Subject::Subjectset(set(1, "a", 0)),
            Subject::ResourceMember(set(0, "a", 0).resource),
            Subject::ResourceMember(set(0, "b", 0).resource),
        ];
        subjects.sort();
        subjects
    }

    #[test]
    fn subject_keys_sort_as_subjects_do() {
        let subjects = subjects();
        let mut keys: Vec<_> = subjects.iter().map(subject).collect();
        keys.sort();
        let sorted: Vec<_> = subjects.iter().map(subject).collect();
        assert_eq!(keys, sorted);
    }

    #[test]
    fn a_span_key_reads_back() {
        for subject_ in subjects() {
            let sk = span(&subject(&subject_), Revision(42));
            assert_eq!(parse_span(&[], &sk).unwrap(), (subject_, Revision(42)));
        }
    }

    #[test]
    fn spans_of_a_subject_sort_by_revision_after_its_pointer() {
        let key = subject(&subjects()[0]);
        let mut keys = vec![
            span(&key, Revision(256)),
            span(&key, Revision(1)),
            key.clone(),
        ];
        keys.sort();
        assert_eq!(
            keys,
            [
                key.clone(),
                span(&key, Revision(1)),
                span(&key, Revision(256))
            ]
        );
    }

    #[test]
    fn a_key_sungma_did_not_write_is_refused() {
        let key = subject(&subjects()[0]);
        for sk in [
            vec![],
            vec![9, 0],
            key.clone(),
            [span(&key, Revision(1)), vec![0]].concat(),
        ] {
            assert!(parse_span(&[], &sk).is_err(), "{sk:?}");
        }
        let text = [
            vec![IDENTITY, 0xff, 0],
            Revision(1).0.to_be_bytes().to_vec(),
        ]
        .concat();
        assert!(parse_span(&[], &text).is_err());
    }

    #[test]
    fn keys_print_as_text() {
        let fact = fact(&set(0, "readme", 2));
        let alice = subject(&Subject::Identity(Identity::new_unchecked("alice")));
        assert_eq!(
            describe(&fact, &span(&alice, Revision(42))),
            "fact 0:readme#2 identity alice @42"
        );
        assert_eq!(
            describe(&fact, &alice),
            "fact 0:readme#2 identity alice pointer"
        );
        let nested = subject(&Subject::Subjectset(set(1, "eng", 3)));
        assert_eq!(
            describe(&fact, &nested),
            "fact 0:readme#2 subjectset 1:eng#3 pointer"
        );
        let member = subject(&Subject::ResourceMember(set(1, "x", 0).resource));
        assert_eq!(
            describe(&fact, &member),
            "fact 0:readme#2 member 1:x pointer"
        );
        let namespace = namespace(NamespaceId(7));
        assert_eq!(describe(&namespace, &latest()), "namespace 7 latest");
        assert_eq!(describe(&namespace, &entry(Revision(3))), "namespace 7 @3");
        assert_eq!(describe(&names(Pool::Relations), &[]), "names 1");
        let (head_pk, _) = head();
        assert_eq!(describe(&head_pk, &[]), "head");
        assert_eq!(describe(&[9, 1], &[7]), "0x0901 0x07");
        assert_eq!(describe(&namespace, &[5]), "namespace 7 0x05");
        assert_eq!(describe(&head_pk, &[0]), "head 0x00");
        assert_eq!(malformed(&head_pk, &[1]).0, "malformed key head 0x01");
    }
}
