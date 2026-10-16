//! Fact rows: keys sort as reads need, and read back.

use sungma::{
    graph::{Resource, Subject, Subjectset},
    id::{NamespaceId, RelationId},
    name::{Identity, ResourceId},
    revision::Revision,
};
use sungma_schema::facts::{Row, covering, pointer, row, span};
use sungma_table::item::{Item, Value};

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

fn written(at: u64) -> Item {
    Item::new().with("written", Value::Number(at))
}

#[test]
fn subject_keys_sort_as_subjects_do() {
    let at = set(0, "readme", 2);
    let sorted: Vec<_> = subjects().iter().map(|s| pointer(&at, s)).collect();
    let mut keys = sorted.clone();
    keys.sort();
    assert_eq!(keys, sorted);
}

#[test]
fn a_fact_s_spans_sort_by_revision_after_its_pointer() {
    let at = set(0, "readme", 2);
    let of = &subjects()[0];
    let mut keys = vec![
        span(&at, of, Revision(256)),
        span(&at, of, Revision(1)),
        pointer(&at, of),
    ];
    keys.sort();
    assert_eq!(
        keys,
        [
            pointer(&at, of),
            span(&at, of, Revision(1)),
            span(&at, of, Revision(256))
        ]
    );
}

#[test]
fn rows_read_back() {
    let at = set(0, "readme", 2);
    for of in subjects() {
        let key = span(&at, &of, Revision(42));
        let read = row(&key.sort, &written(42).with("retired", Value::Number(50))).unwrap();
        assert_eq!(
            read,
            Row::Span {
                subject: of.clone(),
                written: Revision(42),
                retired: Some(Revision(50)),
            }
        );
        let key = pointer(&at, &of);
        let open = Item::new().with("open", Value::Number(42));
        let read = row(&key.sort, &open).unwrap();
        assert_eq!(
            read,
            Row::Pointer {
                subject: of,
                open: Some(Revision(42)),
            }
        );
    }
}

#[test]
fn a_row_sungma_did_not_write_is_refused() {
    let at = set(0, "readme", 2);
    let of = &subjects()[0];
    let span_key = span(&at, of, Revision(1)).sort;
    for (sort, item) in [
        (vec![], Item::new()),
        (vec![9, 0], Item::new()),
        ([span_key.clone(), vec![0]].concat(), written(1)),
        (span_key[..span_key.len() - 1].to_vec(), written(1)),
        (span_key.clone(), Item::new()),
        (span_key.clone(), written(2)),
        (
            span_key.clone(),
            Item::new().with("written", Value::Flag(true)),
        ),
        (vec![1, 0xff, 0, 0], Item::new()),
    ] {
        assert!(row(&sort, &item).is_err(), "{sort:?} {item:?}");
    }
}

#[test]
fn a_span_covers_from_its_writing_until_its_retirement() {
    let alice = Subject::Identity(Identity::new_unchecked("alice"));
    let rows = [
        Row::Pointer {
            subject: alice.clone(),
            open: None,
        },
        Row::Span {
            subject: alice.clone(),
            written: Revision(2),
            retired: Some(Revision(5)),
        },
        Row::Span {
            subject: alice.clone(),
            written: Revision(7),
            retired: None,
        },
    ];
    let held = |at| covering(&rows, Revision(at));
    assert!(held(1).is_empty());
    assert_eq!(held(2), [(alice.clone(), Revision(2))]);
    assert_eq!(held(4), [(alice.clone(), Revision(2))]);
    assert!(held(5).is_empty());
    assert!(held(6).is_empty());
    assert_eq!(held(7), [(alice, Revision(7))]);
}
