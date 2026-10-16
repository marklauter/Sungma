//! What the write context and Check's reader do over one edge beyond the
//! shared store tests: writers with clocks of their own, the unit of work's
//! limit, names a version needs, races between writers, and rows Sungma
//! didn't write. Each test runs on the memory edge and on DynamoDB Local.

mod common;

use std::sync::Arc;

use sungma::{
    graph::{Fact, Resource, Subject, Subjectset},
    id::{NamespaceId, RelationId},
    name::{Identity, NamespaceName, RelationName, ResourceId},
    namespace::Namespace,
    revision::Revision,
    rewrite::Rewrite::This,
    store::Pool,
};
use sungma_check::{
    rows::RowReader,
    store::{FactReader, NamespaceReader},
};
use sungma_clock::DevClock;
use sungma_dynamodb::edge::DynamoEdge;
use sungma_edge::{
    item::{Item, Value},
    key::{Key, encode},
    memory::{MemorySequence, MemoryTable},
    read::{Consistency, ItemReader},
    sequence::Sequence,
    write::{ItemWriter, Step, Work},
};
use sungma_names::{read::NameRows, write::NameRowWriter};
use sungma_write::{
    rows::RowWriter,
    store::{
        FactWrite::{Insert, Retire},
        FactWriteError, FactWriter, NameWriter, NamespaceWrite, NamespaceWriter,
    },
};

type Print = fn(&NamespaceName, &Namespace<RelationName>) -> Result<String, String>;
type Parse = fn(&str) -> Result<(NamespaceName, Namespace<RelationName>), String>;

fn print(name: &NamespaceName, relations: &Namespace<RelationName>) -> Result<String, String> {
    sungma_lang::namespace::print(name, relations).map_err(|error| error.to_string())
}

fn parse(document: &str) -> Result<(NamespaceName, Namespace<RelationName>), String> {
    sungma_lang::namespace::parse(document)
        .map(|document| (document.namespace, document.relations))
        .map_err(|errors| format!("{errors:?}"))
}

/// An edge and its sequence, shared by every writer and reader a test makes.
trait Edge: ItemReader + ItemWriter + Sequence + Clone + Send + Sync + 'static {}

impl<T: ItemReader + ItemWriter + Sequence + Clone + Send + Sync + 'static> Edge for T {}

/// The memory table and sequence as one edge.
#[derive(Clone)]
struct Memory(Arc<MemoryTable>, Arc<MemorySequence>);

impl ItemReader for Memory {
    async fn get(
        &self,
        key: &Key,
        consistency: Consistency,
    ) -> Result<Option<Item>, sungma_edge::EdgeError> {
        self.0.get(key, consistency).await
    }

    async fn query(
        &self,
        query: &sungma_edge::read::Query,
        consistency: Consistency,
    ) -> Result<Vec<(Vec<u8>, Item)>, sungma_edge::EdgeError> {
        self.0.query(query, consistency).await
    }
}

impl ItemWriter for Memory {
    fn max_steps(&self) -> usize {
        self.0.max_steps()
    }

    async fn commit(&self, work: Work) -> Result<(), sungma_edge::write::CommitError> {
        self.0.commit(work).await
    }
}

impl Sequence for Memory {
    async fn next(
        &self,
        key: &Key,
        count: u64,
    ) -> Result<std::ops::Range<u64>, sungma_edge::EdgeError> {
        self.1.next(key, count).await
    }
}

fn memory() -> Memory {
    Memory(
        Arc::new(MemoryTable::new()),
        Arc::new(MemorySequence::new()),
    )
}

async fn dynamo() -> DynamoEdge {
    DynamoEdge::new(common::table().await)
}

/// A writer with a clock of its own, as a node of its own has.
fn writer<E: Edge>(edge: &E, clock: DevClock) -> RowWriter<E, E, DevClock, NameRows<E>, Print> {
    let names = NameRows::new(edge.clone(), Consistency::Strong);
    RowWriter::new(edge.clone(), edge.clone(), clock, names, print as Print)
}

fn reader<E: Edge>(edge: &E) -> RowReader<E, NameRows<E>, Parse> {
    let names = NameRows::new(edge.clone(), Consistency::Eventual);
    RowReader::new(edge.clone(), names, parse as Parse, 16)
}

fn names<E: Edge>(edge: &E) -> NameRowWriter<E, E, E> {
    NameRowWriter::new(edge.clone(), edge.clone(), edge.clone())
}

const DOC: NamespaceId = NamespaceId(0);
const OWNER: RelationId = RelationId(0);
const OTHER: RelationId = RelationId(1);

fn declaring(relation: RelationId) -> Namespace {
    Namespace::new(vec![(relation, This)]).unwrap()
}

/// Interns `doc`, `owner` and `other`, and creates `doc` declaring `owner`.
async fn declared<E: Edge>(edge: &E) -> Revision {
    let names = names(edge);
    for (pool, name, id) in [
        (Pool::Namespaces, "doc", DOC.0),
        (Pool::Relations, "owner", OWNER.0),
        (Pool::Relations, "other", OTHER.0),
    ] {
        names.insert_if_absent(pool, name, id).await.unwrap();
    }
    writer(edge, DevClock::default())
        .write_namespace(DOC, NamespaceWrite::Create(declaring(OWNER)))
        .await
        .unwrap()
}

fn owner(id: &str, who: &str) -> Fact {
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
async fn a_writer_behind_the_head_catches_up<E: Edge>(edge: E) {
    declared(&edge).await;
    let ahead = writer(&edge, DevClock::default());
    // Further behind than sixteen tries a revision apart would reach.
    for _ in 0..20 {
        ahead.write(&[]).await.unwrap();
    }
    let behind = writer(&edge, DevClock::default());
    let landed = behind.write(&[Insert(owner("a", "alice"))]).await.unwrap();
    assert_eq!(landed, Revision(22));
    let reader = reader(&edge);
    assert_eq!(reader.head().await.unwrap(), Revision(22));
    let set = owner("a", "alice").subjectset;
    assert_eq!(reader.subjects(&set, Revision(21)).await.unwrap(), []);
    assert_eq!(reader.subjects(&set, Revision(22)).await.unwrap().len(), 1);
    let version = writer(&edge, DevClock::default())
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
    let names = NameRows::new(edge.clone(), Consistency::Strong);
    let opened = RowWriter::dev(edge.clone(), edge, names, print as Print)
        .await
        .unwrap();
    assert_eq!(opened.write(&[]).await.unwrap(), Revision(24));
}

/// A unit of work holds 100 rows; a larger write is refused before it's
/// committed, takes no revision, and stores nothing.
async fn a_write_past_one_unit_of_work_is_refused<E: Edge>(edge: E) {
    declared(&edge).await;
    let writer = writer(&edge, DevClock::default());
    let reader = reader(&edge);
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
            refusal
                .to_string()
                .contains("more than one unit of work holds"),
            "{refusal}"
        );
        assert_eq!(reader.head().await.unwrap(), head);
        head = Revision(head.0 + 1);
        assert_eq!(writer.write(fits).await.unwrap(), head);
    }
}

/// A namespace write prints its document, so it needs a name for the
/// namespace and each relation.
async fn a_namespace_without_names_is_refused<E: Edge>(edge: E) {
    let writer = writer(&edge, DevClock::default());
    let refused = writer
        .write_namespace(DOC, NamespaceWrite::Create(declaring(OWNER)))
        .await
        .unwrap_err();
    assert_eq!(
        refused.to_string(),
        "store failure: namespace 0 has no interned name"
    );
    let names = names(&edge);
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
        "store failure: relation 0 has no interned name"
    );
}

/// Writers each with a clock of its own, all based on one version, race
/// for it: the unit of work's conditions let exactly one through.
async fn of_writers_on_one_version_exactly_one_succeeds<E: Edge>(make: impl AsyncMake<E>) {
    for _ in 0..5 {
        let edge = make.make().await;
        let first = declared(&edge).await;
        let racers: Vec<_> = (0..4)
            .map(|_| {
                let node = writer(&edge, DevClock::default());
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
async fn an_insert_racing_a_drop_lands_before_it_or_not_at_all<E: Edge>(make: impl AsyncMake<E>) {
    for _ in 0..10 {
        let edge = make.make().await;
        let first = declared(&edge).await;
        // Far ahead, so the head never refuses the insert.
        let facts = writer(&edge, DevClock::starting_after(Revision(1000)));
        let namespaces = writer(&edge, DevClock::default());
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
        let reader = reader(&edge);
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

/// Rows Sungma didn't write are refused when read.
async fn a_row_sungma_did_not_write_is_refused<E: Edge>(edge: E) {
    declared(&edge).await;
    let insert = |key: Key, item: Item| Work(vec![Step::Insert { key, item }]);
    let set = owner("a", "alice").subjectset;
    let partition = sungma_schema::facts::partition(&set);
    let rows = [
        // A sort key that names no subject.
        (encode(9u8), Item::new()),
        // A span whose written revision isn't its key's.
        (
            encode(((1u8, "x"), 5u64)),
            Item::new().with("written", Value::Number(6)),
        ),
    ];
    let reader = reader(&edge);
    for (sort, item) in rows {
        let other = owner("b", "x").subjectset;
        let key = Key {
            partition: if sort.len() == 1 {
                partition.clone()
            } else {
                sungma_schema::facts::partition(&other)
            },
            sort,
        };
        edge.commit(insert(key, item)).await.unwrap();
    }
    assert!(reader.subjects(&set, Revision(10)).await.is_err());
    let other = owner("b", "x").subjectset;
    assert!(reader.subjects(&other, Revision(10)).await.is_err());
    // A latest row without a revision.
    let ghost = NamespaceId(9);
    edge.commit(insert(
        sungma_schema::namespaces::latest_key(ghost),
        Item::new().with("document", Value::Text("{}".to_owned())),
    ))
    .await
    .unwrap();
    assert!(reader.version(ghost, Revision(10)).await.is_err());
}

/// Makes a fresh edge, for tests that make several.
trait AsyncMake<E>: Send + Sync {
    fn make(&self) -> impl std::future::Future<Output = E> + Send;
}

struct MakeMemory;

impl AsyncMake<Memory> for MakeMemory {
    async fn make(&self) -> Memory {
        memory()
    }
}

struct MakeDynamo;

impl AsyncMake<DynamoEdge> for MakeDynamo {
    async fn make(&self) -> DynamoEdge {
        dynamo().await
    }
}

macro_rules! on_both {
    ($($test:ident),* $(,)?) => {
        mod on_memory {
            $(
                #[tokio::test]
                async fn $test() {
                    super::$test(super::memory()).await;
                }
            )*
        }

        mod on_dynamodb {
            $(
                #[tokio::test]
                async fn $test() {
                    super::$test(super::dynamo().await).await;
                }
            )*
        }
    };
}

on_both!(
    a_writer_behind_the_head_catches_up,
    a_write_past_one_unit_of_work_is_refused,
    a_namespace_without_names_is_refused,
    a_row_sungma_did_not_write_is_refused,
);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn of_writers_on_one_version_exactly_one_succeeds_on_memory() {
    of_writers_on_one_version_exactly_one_succeeds(MakeMemory).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn of_writers_on_one_version_exactly_one_succeeds_on_dynamodb() {
    of_writers_on_one_version_exactly_one_succeeds(MakeDynamo).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_insert_racing_a_drop_lands_before_it_or_not_at_all_on_memory() {
    an_insert_racing_a_drop_lands_before_it_or_not_at_all(MakeMemory).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_insert_racing_a_drop_lands_before_it_or_not_at_all_on_dynamodb() {
    an_insert_racing_a_drop_lands_before_it_or_not_at_all(MakeDynamo).await;
}
