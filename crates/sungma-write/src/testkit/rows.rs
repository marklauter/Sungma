//! What the row writer does over a table beyond the shared store tests:
//! writers with clocks of their own, the unit of work's limit, names a
//! version needs, races between writers, and spans their pointers disagree
//! with. A table's crate calls [`row_writer_tests!`](crate::row_writer_tests)
//! with a [`Make`] for its table.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use sungma::{
    graph::{Fact, Resource, Subject, Subjectset},
    id::{NamespaceId, RelationId},
    name::{Identity, ResourceId},
    namespace::Namespace,
    revision::Revision,
    rewrite::Rewrite::This,
};
use sungma_check::{
    rows::RowReader,
    store::{FactReader, NamespaceReader},
};
use sungma_clock::DevClock;
use sungma_names::store::{NameWriter, Pool};
use sungma_names::{read::NameRows, write::NameRowWriter};
use sungma_table::{
    TableError,
    item::{Item, Value},
    key::Key,
    memory::{MemorySequence, MemoryTable},
    read::{Consistency, ItemReader, Query},
    sequence::Sequence,
    write::{CommitError, ItemWriter, Step, Work},
};

use super::{Parse, Print, parse, print};
use crate::{
    rows::RowWriter,
    store::{
        FactWrite::{Insert, Retire},
        FactWriteError, FactWriter, NamespaceWrite, NamespaceWriter,
    },
};

/// Writes a test for each function here, on the tables `$make` makes, a
/// [`Make`].
#[macro_export]
macro_rules! row_writer_tests {
    ($make:expr) => {
        $crate::row_writer_tests!(@each $make;
            a_writer_behind_the_head_catches_up,
            a_writer_that_keeps_losing_races_catches_up_each_time,
            a_write_past_one_unit_of_work_is_refused,
            a_namespace_without_names_is_refused,
            a_span_its_pointer_disagrees_with_is_reported,
        );

        #[::tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn of_writers_on_one_version_exactly_one_succeeds() {
            $crate::testkit::rows::of_writers_on_one_version_exactly_one_succeeds($make).await;
        }

        #[::tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn an_insert_racing_a_drop_lands_before_it_or_not_at_all() {
            $crate::testkit::rows::an_insert_racing_a_drop_lands_before_it_or_not_at_all($make)
                .await;
        }
    };
    (@each $make:expr; $($test:ident,)*) => {
        $(
            #[::tokio::test]
            async fn $test() {
                use $crate::testkit::rows::Make as _;
                $crate::testkit::rows::$test($make.make().await).await;
            }
        )*
    };
}

/// A table and its sequence, shared by every writer and reader a test makes.
pub trait Table: ItemReader + ItemWriter + Sequence + Clone + Send + Sync + 'static {}

impl<T: ItemReader + ItemWriter + Sequence + Clone + Send + Sync + 'static> Table for T {}

/// Makes a fresh, empty table.
pub trait Make<E>: Send + Sync {
    fn make(&self) -> impl std::future::Future<Output = E> + Send;
}

/// The memory table and sequence as one table.
#[derive(Clone, Default)]
pub struct Memory(Arc<MemoryTable>, Arc<MemorySequence>);

impl ItemReader for Memory {
    async fn get(&self, key: &Key, consistency: Consistency) -> Result<Option<Item>, TableError> {
        self.0.get(key, consistency).await
    }

    async fn query(
        &self,
        query: &Query,
        consistency: Consistency,
    ) -> Result<Vec<(Vec<u8>, Item)>, TableError> {
        self.0.query(query, consistency).await
    }
}

impl ItemWriter for Memory {
    fn max_steps(&self) -> usize {
        self.0.max_steps()
    }

    async fn commit(&self, work: Work) -> Result<(), CommitError> {
        self.0.commit(work).await
    }
}

impl Sequence for Memory {
    async fn next(&self, key: &Key, count: u64) -> Result<std::ops::Range<u64>, TableError> {
        self.1.next(key, count).await
    }
}

/// Makes empty memory tables.
pub struct MakeMemory;

impl Make<Memory> for MakeMemory {
    async fn make(&self) -> Memory {
        Memory::default()
    }
}

/// A writer with a clock of its own, as a node of its own has.
pub fn writer<E: Table>(
    table: &E,
    clock: DevClock,
) -> RowWriter<E, E, DevClock, NameRows<E>, Print> {
    let names = NameRows::new(table.clone(), Consistency::Strong);
    RowWriter::new(table.clone(), table.clone(), clock, names, print as Print)
}

pub fn reader<E: Table>(table: &E) -> RowReader<E, NameRows<E>, Parse> {
    let names = NameRows::new(table.clone(), Consistency::Eventual);
    RowReader::new(table.clone(), names, parse as Parse, 16)
}

fn names<E: Table>(table: &E) -> NameRowWriter<E, E, E> {
    NameRowWriter::new(table.clone(), table.clone(), table.clone())
}

pub const DOC: NamespaceId = NamespaceId(0);
pub const OWNER: RelationId = RelationId(0);
pub const OTHER: RelationId = RelationId(1);

pub fn declaring(relation: RelationId) -> Namespace {
    Namespace::new(vec![(relation, This)]).unwrap()
}

/// Interns `doc`, `owner` and `other`, and creates `doc` declaring `owner`.
pub async fn declared<E: Table>(table: &E) -> Revision {
    let names = names(table);
    for (pool, name, id) in [
        (Pool::Namespaces, "doc", DOC.0),
        (Pool::Relations, "owner", OWNER.0),
        (Pool::Relations, "other", OTHER.0),
    ] {
        names.insert_if_absent(pool, name, id).await.unwrap();
    }
    writer(table, DevClock::default())
        .write_namespace(DOC, NamespaceWrite::Create(declaring(OWNER)))
        .await
        .unwrap()
}

/// `doc:id#owner@who`.
pub fn owner(id: &str, who: &str) -> Fact {
    Fact {
        subjectset: Subjectset {
            resource: Resource {
                namespace: DOC,
                id: ResourceId::new_unchecked(id),
            },
            relation: OWNER,
        },
        subject: Subject::Identity(Identity::new_unchecked(who)),
    }
}

/// A writer whose clock is behind the head is refused, moves its clock
/// past the head, and lands its write after every earlier one.
pub async fn a_writer_behind_the_head_catches_up<E: Table>(table: E) {
    declared(&table).await;
    let ahead = writer(&table, DevClock::default());
    // Further behind than sixteen tries a revision apart would reach.
    for _ in 0..20 {
        ahead.write(&[]).await.unwrap();
    }
    let behind = writer(&table, DevClock::default());
    let landed = behind.write(&[Insert(owner("a", "alice"))]).await.unwrap();
    assert_eq!(landed, Revision(22));
    let reader = reader(&table);
    assert_eq!(reader.head().await.unwrap(), Revision(22));
    let set = owner("a", "alice").subjectset;
    assert_eq!(reader.subjects(&set, Revision(21)).await.unwrap(), []);
    assert_eq!(reader.subjects(&set, Revision(22)).await.unwrap().len(), 1);
    let version = writer(&table, DevClock::default())
        .write_namespace(
            DOC,
            NamespaceWrite::NewVersion {
                based_on: Revision(1),
                namespace: declaring(OWNER),
            },
        )
        .await
        .unwrap();
    assert_eq!(version, Revision(23));
    let names = NameRows::new(table.clone(), Consistency::Strong);
    let opened = RowWriter::dev(table.clone(), table, names, print as Print)
        .await
        .unwrap();
    assert_eq!(opened.write(&[]).await.unwrap(), Revision(24));
}

/// A table where another writer moves the head far ahead just before each
/// of the first `races` commits, so each of them loses its race.
#[derive(Clone)]
struct Racing<E> {
    table: E,
    races: Arc<AtomicUsize>,
}

impl<E: Table> ItemReader for Racing<E> {
    async fn get(&self, key: &Key, consistency: Consistency) -> Result<Option<Item>, TableError> {
        self.table.get(key, consistency).await
    }

    async fn query(
        &self,
        query: &Query,
        consistency: Consistency,
    ) -> Result<Vec<(Vec<u8>, Item)>, TableError> {
        self.table.query(query, consistency).await
    }
}

impl<E: Table> ItemWriter for Racing<E> {
    fn max_steps(&self) -> usize {
        self.table.max_steps()
    }

    async fn commit(&self, work: Work) -> Result<(), CommitError> {
        let racing = self
            .races
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            });
        if racing.is_ok() {
            let head = sungma_schema::head::read(&self.table, Consistency::Strong)
                .await
                .unwrap()
                .unwrap();
            let ahead = sungma_schema::head::step(Some(head), Revision(head.0 + 100));
            self.table.commit(Work(vec![ahead])).await.unwrap();
        }
        self.table.commit(work).await
    }
}

impl<E: Table> Sequence for Racing<E> {
    async fn next(&self, key: &Key, count: u64) -> Result<std::ops::Range<u64>, TableError> {
        self.table.next(key, count).await
    }
}

/// A write whose head step is refused moves its clock past the head
/// before trying again, so each lost race costs one try, and a write that
/// loses ten in a row still lands.
pub async fn a_writer_that_keeps_losing_races_catches_up_each_time<E: Table>(table: E) {
    declared(&table).await;
    let racing = Racing {
        table,
        races: Arc::new(AtomicUsize::new(10)),
    };
    let writer = writer(&racing, DevClock::default());
    let landed = writer.write(&[Insert(owner("a", "alice"))]).await;
    assert_eq!(landed.unwrap(), Revision(1 + 10 * 100 + 1));
}

/// A unit of work holds 100 rows; a larger write is refused before it's
/// committed, takes no revision, and stores nothing.
pub async fn a_write_past_one_unit_of_work_is_refused<E: Table>(table: E) {
    declared(&table).await;
    let writer = writer(&table, DevClock::default());
    let reader = reader(&table);
    let grants = |count: usize| -> Vec<_> {
        (0..count)
            .map(|i| Insert(owner("a", &format!("p{i}"))))
            .collect()
    };
    // The head, a check of `doc`, and a pointer and a span per grant.
    let full = grants(49);
    let mut past = full.clone();
    past.push(Retire(owner("b", "absent")));
    // The head and a pointer check per retirement of an absent fact.
    let retires: Vec<_> = (0..100)
        .map(|i| Retire(owner("b", &format!("p{i}"))))
        .collect();
    let mut head = Revision(1);
    for (refused, fits) in [(&past, &full), (&retires, &retires[..99].to_vec())] {
        let refusal = writer.write(refused).await.unwrap_err();
        assert!(
            refusal.to_string().contains("more than one write holds"),
            "{refusal}"
        );
        assert_eq!(reader.head().await.unwrap(), head);
        head = Revision(head.0 + 1);
        assert_eq!(writer.write(fits).await.unwrap(), head);
    }
    // A pointer and a closed span per retirement of a stored fact, and the
    // head, a check of `doc`, and a pointer and a span for the new grant.
    let mut retires: Vec<_> = (0..49)
        .map(|i| Retire(owner("a", &format!("p{i}"))))
        .collect();
    retires.push(Insert(owner("c", "new")));
    let refusal = writer.write(&retires).await.unwrap_err();
    assert!(
        matches!(refusal, FactWriteError::TooLarge { rows: 102, .. }),
        "{refusal}"
    );
    assert_eq!(reader.head().await.unwrap(), head);
}

/// A namespace write prints its document, so it needs a name for the
/// namespace and each relation.
pub async fn a_namespace_without_names_is_refused<E: Table>(table: E) {
    let writer = writer(&table, DevClock::default());
    let refused = writer
        .write_namespace(DOC, NamespaceWrite::Create(declaring(OWNER)))
        .await
        .unwrap_err();
    assert_eq!(
        refused.to_string(),
        "store failure: Namespaces id 0 has no name"
    );
    let names = names(&table);
    names
        .insert_if_absent(Pool::Namespaces, "doc", 0)
        .await
        .unwrap();
    let refused = writer
        .write_namespace(DOC, NamespaceWrite::Create(declaring(OWNER)))
        .await
        .unwrap_err();
    assert_eq!(
        refused.to_string(),
        "store failure: Relations id 0 has no name"
    );
}

/// A span whose row disagrees with its fact's pointer can't be closed by
/// trying again: the write reports the row rather than a lost race.
pub async fn a_span_its_pointer_disagrees_with_is_reported<E: Table>(table: E) {
    declared(&table).await;
    let writer = writer(&table, DevClock::default());
    let fact = owner("a", "alice");
    let written = writer.write(&[Insert(fact.clone())]).await.unwrap();
    // Retire the span behind the pointer's back.
    let span = sungma_schema::facts::span(&fact.subjectset, &fact.subject, written);
    let retire = Step::Update {
        key: span,
        set: Item::new().with("retired", Value::Number(written.0 + 1)),
        remove: vec![],
        expect: vec![],
    };
    table.commit(Work(vec![retire])).await.unwrap();
    let refused = writer.write(&[Retire(fact)]).await.unwrap_err();
    let message = refused.to_string();
    assert!(!message.contains("races"), "{message}");
    assert!(message.contains("span"), "{message}");
}

/// Writers each with a clock of its own, all based on one version, race
/// for it: the unit of work's conditions let exactly one through.
pub async fn of_writers_on_one_version_exactly_one_succeeds<E: Table>(make: impl Make<E>) {
    for _ in 0..5 {
        let table = make.make().await;
        let first = declared(&table).await;
        let racers: Vec<_> = (0..4)
            .map(|_| {
                let node = writer(&table, DevClock::default());
                tokio::spawn(async move {
                    let write = NamespaceWrite::NewVersion {
                        based_on: first,
                        namespace: declaring(OWNER),
                    };
                    node.write_namespace(DOC, write).await.is_ok()
                })
            })
            .collect();
        let mut stored = 0;
        for racer in racers {
            stored += usize::from(racer.await.unwrap());
        }
        assert_eq!(stored, 1);
    }
}

/// A fact write is judged by the latest versions when it lands: an insert
/// under a relation another writer drops at the same time lands before the
/// drop, or is refused.
pub async fn an_insert_racing_a_drop_lands_before_it_or_not_at_all<E: Table>(make: impl Make<E>) {
    for _ in 0..10 {
        let table = make.make().await;
        let first = declared(&table).await;
        // Far ahead, so the head never refuses the insert.
        let facts = writer(&table, DevClock::starting_after(Revision(1000)));
        let namespaces = writer(&table, DevClock::default());
        let grants: Vec<_> = (0..40)
            .map(|i| Insert(owner("a", &format!("p{i}"))))
            .collect();
        let insert = tokio::spawn(async move { facts.write(&grants).await });
        let drop = tokio::spawn(async move {
            let write = NamespaceWrite::NewVersion {
                based_on: first,
                namespace: declaring(OTHER),
            };
            namespaces.write_namespace(DOC, write).await.unwrap()
        });
        let (inserted, dropped) = (insert.await.unwrap(), drop.await.unwrap());
        let reader = reader(&table);
        let latest = reader.version(DOC, Revision(2000)).await.unwrap().unwrap();
        assert!(latest.declares(OTHER));
        match inserted {
            Ok(revision) => {
                let at_insert = reader.version(DOC, revision).await.unwrap().unwrap();
                assert!(
                    at_insert.declares(OWNER),
                    "{revision:?} landed after the drop at {dropped:?}"
                );
            }
            Err(refused) => assert!(
                matches!(refused, FactWriteError::Undeclared(_)),
                "{refused}"
            ),
        }
    }
}
