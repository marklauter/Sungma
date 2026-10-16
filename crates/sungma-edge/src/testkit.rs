//! The tests every edge passes, so a test of the memory table holds for
//! DynamoDB too. Each takes a fresh, empty edge.

use crate::{
    item::{Item, Value},
    key::{Key, encode},
    read::{Consistency, ItemReader, Query, Range},
    sequence::Sequence,
    write::{CommitError, Expect, ItemWriter, Step, Work},
};

/// Every read, strong and eventual: an edge that reads strongly when it's
/// asked for eventual reads still passes.
const READS: [Consistency; 2] = [Consistency::Strong, Consistency::Eventual];

fn item(n: u64) -> Item {
    Item::new().with("n", Value::Number(n))
}

fn key(partition: &str, sort: u64) -> Key {
    Key::new(partition, sort)
}

async fn commit<E: ItemWriter>(edge: &E, steps: Vec<Step>) -> Result<(), CommitError> {
    edge.commit(Work(steps)).await
}

async fn insert<E: ItemWriter>(edge: &E, key: Key, item: Item) {
    commit(edge, vec![Step::Insert { key, item }])
        .await
        .unwrap();
}

/// An inserted item reads back; a key never written reads as none.
pub async fn an_item_reads_back<E: ItemReader + ItemWriter>(edge: E) {
    let stored = Item::new()
        .with("n", Value::Number(7))
        .with("t", Value::Text("seven".to_owned()))
        .with("f", Value::Flag(true))
        .with("m", Value::Map([("1".to_owned(), Value::Number(1))].into()));
    insert(&edge, key("p", 1), stored.clone()).await;
    for read in READS {
        assert_eq!(
            edge.get(&key("p", 1), read).await.unwrap(),
            Some(stored.clone())
        );
        assert_eq!(edge.get(&key("p", 2), read).await.unwrap(), None);
        assert_eq!(edge.get(&key("q", 1), read).await.unwrap(), None);
    }
}

/// A query takes its partition's items in sort key order, within its
/// range, either way, up to its limit.
pub async fn a_query_takes_a_run_of_one_partition<E: ItemReader + ItemWriter>(edge: E) {
    for sort in [3, 1, 2, 10] {
        insert(&edge, key("p", sort), item(sort)).await;
    }
    insert(&edge, key("pp", 0), item(0)).await;
    insert(&edge, key("o", 0), item(0)).await;
    let numbers = |run: Vec<(Vec<u8>, Item)>| {
        run.into_iter()
            .map(|(sort, item)| {
                assert_eq!(sort, encode(item.required_number("n").unwrap()));
                item.required_number("n").unwrap()
            })
            .collect::<Vec<_>>()
    };
    let partition = encode("p");
    let cases = [
        (Range::All, false, None, vec![1, 2, 3, 10]),
        (Range::All, true, None, vec![10, 3, 2, 1]),
        (Range::All, false, Some(2), vec![1, 2]),
        (Range::All, true, Some(1), vec![10]),
        (
            Range::Between(encode(2u64), encode(3u64)),
            false,
            None,
            vec![2, 3],
        ),
        (
            Range::Between(encode(0u64), encode(9u64)),
            true,
            Some(1),
            vec![3],
        ),
        (
            Range::Prefix(vec![0, 0, 0, 0, 0, 0, 0]),
            false,
            None,
            vec![1, 2, 3, 10],
        ),
        (Range::Prefix(encode(2u64)), false, None, vec![2]),
        (Range::Prefix(vec![1]), false, None, vec![]),
    ];
    for (range, descending, limit, expected) in cases {
        let query = Query {
            partition: partition.clone(),
            range: range.clone(),
            descending,
            limit,
        };
        for read in READS {
            let run = edge.query(&query, read).await.unwrap();
            assert_eq!(numbers(run), expected, "{range:?} {descending} {limit:?}");
        }
    }
    let empty = edge
        .query(&Query::partition(encode("none")), Consistency::Strong)
        .await
        .unwrap();
    assert!(empty.is_empty());
}

/// Each kind of step holds or refuses as it says, and a refused unit of
/// work names every step that failed and writes nothing.
pub async fn work_commits_whole_or_not_at_all<E: ItemReader + ItemWriter>(edge: E) {
    insert(&edge, key("p", 1), item(1)).await;
    insert(&edge, key("p", 4), item(4)).await;
    let n = |n| Expect::Equals("n".to_owned(), Value::Number(n));
    let refused = commit(
        &edge,
        vec![
            Step::Insert {
                key: key("p", 2),
                item: item(2),
            },
            Step::Insert {
                key: key("p", 4),
                item: item(9),
            },
            Step::Update {
                key: key("p", 1),
                set: item(5),
                remove: vec![],
                expect: vec![n(2)],
            },
            Step::Check {
                key: key("p", 3),
                expect: vec![],
            },
            Step::Absent { key: key("q", 1) },
        ],
    )
    .await;
    assert_eq!(refused, Err(CommitError::Refused(vec![1, 2, 3])));
    assert_eq!(
        edge.get(&key("p", 2), Consistency::Strong).await.unwrap(),
        None
    );
    assert_eq!(
        edge.get(&key("p", 1), Consistency::Strong).await.unwrap(),
        Some(item(1))
    );

    let set = Item::new()
        .with("n", Value::Number(5))
        .with("t", Value::Text("x".to_owned()));
    commit(
        &edge,
        vec![
            Step::Insert {
                key: key("p", 2),
                item: item(2),
            },
            Step::Update {
                key: key("p", 1),
                set,
                remove: vec![],
                expect: vec![n(1), Expect::Missing("t".to_owned())],
            },
            Step::Absent { key: key("q", 1) },
        ],
    )
    .await
    .unwrap();
    let updated = edge
        .get(&key("p", 1), Consistency::Strong)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.number("n").unwrap(), Some(5));
    assert_eq!(updated.text("t").unwrap(), Some("x"));

    // An update keeps the attributes it doesn't set.
    let flag = Item::new().with("f", Value::Flag(true));
    commit(
        &edge,
        vec![Step::Update {
            key: key("p", 1),
            set: flag,
            remove: vec![],
            expect: vec![],
        }],
    )
    .await
    .unwrap();
    let updated = edge
        .get(&key("p", 1), Consistency::Strong)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.number("n").unwrap(), Some(5));
    assert!(updated.flag("f").unwrap());

    // An update removes the attributes it names, and one that changes
    // nothing only checks.
    for remove in [vec!["t".to_owned(), "f".to_owned()], vec![]] {
        commit(
            &edge,
            vec![Step::Update {
                key: key("p", 1),
                set: Item::new(),
                remove,
                expect: vec![n(5)],
            }],
        )
        .await
        .unwrap();
    }
    let updated = edge
        .get(&key("p", 1), Consistency::Strong)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated, item(5));

    let checked = commit(
        &edge,
        vec![
            Step::Check {
                key: key("p", 1),
                expect: vec![n(5), Expect::Missing("m".to_owned())],
            },
            Step::Check {
                key: key("p", 2),
                expect: vec![Expect::Missing("n".to_owned())],
            },
        ],
    )
    .await;
    assert_eq!(checked, Err(CommitError::Refused(vec![1])));
}

/// Work past the most steps, or with two steps on one key, is refused
/// before anything is written.
pub async fn unwritable_work_is_refused<E: ItemReader + ItemWriter>(edge: E) {
    let most = edge.max_steps() as u64;
    let steps = |count: u64| {
        (0..count)
            .map(|n| Step::Insert {
                key: key("p", n),
                item: item(n),
            })
            .collect::<Vec<_>>()
    };
    let refused = commit(&edge, steps(most + 1)).await;
    assert!(
        matches!(refused, Err(CommitError::Unwritable(_))),
        "{refused:?}"
    );
    assert_eq!(
        edge.get(&key("p", 0), Consistency::Strong).await.unwrap(),
        None
    );
    commit(&edge, steps(most)).await.unwrap();

    let twice = commit(
        &edge,
        vec![
            Step::Insert {
                key: key("q", 0),
                item: item(0),
            },
            Step::Absent { key: key("q", 0) },
        ],
    )
    .await;
    assert!(
        matches!(twice, Err(CommitError::Unwritable(_))),
        "{twice:?}"
    );
    assert_eq!(
        edge.get(&key("q", 0), Consistency::Strong).await.unwrap(),
        None
    );
}

/// A sequence hands out each number once, from 0, per counter.
pub async fn a_sequence_hands_out_each_number_once<E: Sequence>(edge: E) {
    let a = key("seq", 0);
    let b = key("seq", 1);
    assert_eq!(edge.next(&a, 3).await.unwrap(), 0..3);
    assert_eq!(edge.next(&a, 2).await.unwrap(), 3..5);
    assert_eq!(edge.next(&b, 1).await.unwrap(), 0..1);
    assert_eq!(edge.next(&a, 0).await.unwrap(), 5..5);
    assert_eq!(edge.next(&a, u64::MAX).await.unwrap(), 5..u64::MAX);
    assert_eq!(edge.next(&a, 1).await.unwrap(), u64::MAX..u64::MAX);
}

/// Runs every test, each on an edge `fresh` makes, and the sequence test
/// on a sequence `sequence` makes.
#[macro_export]
macro_rules! edge_tests {
    ($fresh:expr, $sequence:expr) => {
        #[tokio::test]
        async fn an_item_reads_back() {
            $crate::testkit::an_item_reads_back($fresh).await;
        }

        #[tokio::test]
        async fn a_query_takes_a_run_of_one_partition() {
            $crate::testkit::a_query_takes_a_run_of_one_partition($fresh).await;
        }

        #[tokio::test]
        async fn work_commits_whole_or_not_at_all() {
            $crate::testkit::work_commits_whole_or_not_at_all($fresh).await;
        }

        #[tokio::test]
        async fn unwritable_work_is_refused() {
            $crate::testkit::unwritable_work_is_refused($fresh).await;
        }

        #[tokio::test]
        async fn a_sequence_hands_out_each_number_once() {
            $crate::testkit::a_sequence_hands_out_each_number_once($sequence).await;
        }
    };
}
