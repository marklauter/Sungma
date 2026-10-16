//! Fact rows. A fact's subjectset is its partition; under it, each fact has
//! a pointer row keyed by its subject, and a span row per span keyed by its
//! subject and then the revision that wrote it, so a fact's spans sort by
//! revision after its pointer. A subject's key sorts as subjects do:
//! identities, then subjectsets, then resource members.
//!
//! A span row records when it was written and, once closed, retired. A
//! pointer records the revision that wrote the fact's open span, and none
//! when no span is open; writes are conditioned on it.

use sungma::{
    graph::{Resource, Subject, Subjectset},
    id::{NamespaceId, RelationId},
    name::{Identity, ResourceId},
    revision::Revision,
    store::StoreError,
};
use sungma_edge::{
    item::{Item, Value},
    key::{Key, Part, Parts, encode},
    read::{Consistency, ItemReader, Query, Range},
    write::{Expect, Step},
};

use crate::{FACT, failed};

const IDENTITY: u8 = 1;
const SUBJECTSET: u8 = 2;
const RESOURCE_MEMBER: u8 = 3;

const WRITTEN: &str = "written";
const RETIRED: &str = "retired";
const OPEN: &str = "open";

/// A fact's partition: its subjectset.
pub fn partition(set: &Subjectset) -> Vec<u8> {
    encode((
        FACT,
        set.resource.namespace.0,
        set.relation.0,
        set.resource.id.as_str(),
    ))
}

struct SubjectPart<'a>(&'a Subject);

impl Part for SubjectPart<'_> {
    fn encode(&self, key: &mut Vec<u8>) {
        match self.0 {
            Subject::Identity(identity) => (IDENTITY, identity.as_str()).encode(key),
            Subject::Subjectset(set) => (
                SUBJECTSET,
                set.resource.namespace.0,
                set.resource.id.as_str(),
                set.relation.0,
            )
                .encode(key),
            Subject::ResourceMember(resource) => {
                (RESOURCE_MEMBER, resource.namespace.0, resource.id.as_str()).encode(key)
            }
        }
    }
}

fn subject(parts: &mut Parts<'_>) -> Option<Subject> {
    let resource = |parts: &mut Parts<'_>| {
        Some(Resource {
            namespace: NamespaceId(parts.u32()?),
            id: ResourceId::new_unchecked(parts.text()?),
        })
    };
    Some(match parts.byte()? {
        IDENTITY => Subject::Identity(Identity::new_unchecked(parts.text()?)),
        SUBJECTSET => {
            let resource = resource(parts)?;
            Subject::Subjectset(Subjectset {
                resource,
                relation: RelationId(parts.u32()?),
            })
        }
        RESOURCE_MEMBER => Subject::ResourceMember(resource(parts)?),
        _ => return None,
    })
}

/// The fact's pointer row.
pub fn pointer(set: &Subjectset, of: &Subject) -> Key {
    Key {
        partition: partition(set),
        sort: encode(SubjectPart(of)),
    }
}

/// The fact's span row written at `written`.
pub fn span(set: &Subjectset, of: &Subject, written: Revision) -> Key {
    Key {
        partition: partition(set),
        sort: encode((SubjectPart(of), written.0)),
    }
}

/// One row under a subjectset.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Row {
    Pointer {
        subject: Subject,
        open: Option<Revision>,
    },
    Span {
        subject: Subject,
        written: Revision,
        retired: Option<Revision>,
    },
}

/// The row a query gave, refused when Sungma didn't write it.
pub fn row(sort: &[u8], item: &Item) -> Result<Row, StoreError> {
    let malformed = || StoreError(format!("a fact row with a malformed key {sort:02x?}"));
    let mut parts = Parts::new(sort);
    let of = subject(&mut parts).ok_or_else(malformed)?;
    let revision = |name| item.number(name).map(|n| n.map(Revision)).map_err(failed);
    if parts.end(()).is_some() {
        return Ok(Row::Pointer {
            subject: of,
            open: revision(OPEN)?,
        });
    }
    let written = parts.u64().map(Revision).ok_or_else(malformed)?;
    parts.end(()).ok_or_else(malformed)?;
    if revision(WRITTEN)? != Some(written) {
        return Err(StoreError(format!(
            "a span keyed at {written:?} doesn't record that it was written then"
        )));
    }
    Ok(Row::Span {
        subject: of,
        written,
        retired: revision(RETIRED)?,
    })
}

/// Every row under `set`, or only the rows of subjectset subjects, or of
/// one subject.
pub async fn rows<R: ItemReader>(
    items: &R,
    set: &Subjectset,
    of: Rows<'_>,
    consistency: Consistency,
) -> Result<Vec<Row>, StoreError> {
    let range = match of {
        Rows::All => Range::All,
        Rows::Subjectsets => Range::Prefix(vec![SUBJECTSET]),
        Rows::Of(subject) => Range::Prefix(encode(SubjectPart(subject))),
    };
    let query = Query {
        partition: partition(set),
        range,
        descending: false,
        limit: None,
    };
    let found = items.query(&query, consistency).await.map_err(failed)?;
    found.iter().map(|(sort, item)| row(sort, item)).collect()
}

/// Which rows of a subjectset [`rows`] reads.
#[derive(Clone, Copy, Debug)]
pub enum Rows<'a> {
    All,
    Subjectsets,
    Of(&'a Subject),
}

/// Each subject whose span covers `revision`, with the revision that wrote
/// that span, in subject order.
pub fn covering(rows: &[Row], revision: Revision) -> Vec<(Subject, Revision)> {
    rows.iter()
        .filter_map(|row| match row {
            Row::Span {
                subject,
                written,
                retired,
            } if *written <= revision && retired.is_none_or(|retired| revision < retired) => {
                Some((subject.clone(), *written))
            }
            _ => None,
        })
        .collect()
}

/// A fact's pointer: `None` when the fact was never stored, and otherwise
/// the revision that wrote its open span, if one is open.
pub async fn read_pointer<R: ItemReader>(
    items: &R,
    set: &Subjectset,
    of: &Subject,
    consistency: Consistency,
) -> Result<Option<Option<Revision>>, StoreError> {
    let row = items
        .get(&pointer(set, of), consistency)
        .await
        .map_err(failed)?;
    row.map(|row| {
        row.number(OPEN)
            .map(|open| open.map(Revision))
            .map_err(failed)
    })
    .transpose()
}

fn holds(open: Option<Revision>) -> Expect {
    match open {
        Some(open) => Expect::Equals(OPEN.to_owned(), Value::Number(open.0)),
        None => Expect::Missing(OPEN.to_owned()),
    }
}

/// A step that leaves the pointer as `seen`, what the writer read, or
/// refuses.
pub fn pointer_unchanged(set: &Subjectset, of: &Subject, seen: Option<Option<Revision>>) -> Step {
    let key = pointer(set, of);
    match seen {
        None => Step::Absent { key },
        Some(open) => Step::Check {
            key,
            expect: vec![holds(open)],
        },
    }
}

/// A step that moves the pointer from `seen` to `open`.
pub fn pointer_moved(
    set: &Subjectset,
    of: &Subject,
    seen: Option<Option<Revision>>,
    open: Option<Revision>,
) -> Step {
    let key = pointer(set, of);
    let set_open = |item: Item| match open {
        Some(open) => item.with(OPEN, Value::Number(open.0)),
        None => item,
    };
    match seen {
        None => Step::Insert {
            key,
            item: set_open(Item::new()),
        },
        Some(was) => Step::Update {
            key,
            set: set_open(Item::new()),
            remove: if open.is_none() {
                vec![OPEN.to_owned()]
            } else {
                Vec::new()
            },
            expect: vec![holds(was)],
        },
    }
}

/// A step that opens a span at `at`.
pub fn span_opened(set: &Subjectset, of: &Subject, at: Revision) -> Step {
    Step::Insert {
        key: span(set, of, at),
        item: Item::new().with(WRITTEN, Value::Number(at.0)),
    }
}

/// A step that closes the span written at `written`, at `at`.
pub fn span_closed(set: &Subjectset, of: &Subject, written: Revision, at: Revision) -> Step {
    Step::Update {
        key: span(set, of, written),
        set: Item::new().with(RETIRED, Value::Number(at.0)),
        remove: Vec::new(),
        expect: vec![Expect::Missing(RETIRED.to_owned())],
    }
}
