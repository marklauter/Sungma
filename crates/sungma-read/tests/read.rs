//! The API's reads over rows the write context wrote, on the memory table.

use std::num::NonZeroU32;

use sungma::{
    graph::Fact,
    name::{FactName, NamespaceName, RelationName, SubjectName, SubjectsetName},
    namespace::Namespace,
    revision::Revision,
    rewrite::Rewrite,
};
use sungma_clock::DevClock;
use sungma_lang::namespace;
use sungma_names::intern::LeasingInterner;
use sungma_names::{read::NameRows, write::NameRowWriter};
use sungma_names::{resolve, store::ReadingDictionary};
use sungma_read::{ReadError, facts::FactReadService, namespaces::NamespaceReadService};
use sungma_table::{
    memory::{MemorySequence, MemoryTable},
    read::Consistency,
};
use sungma_write::{
    rows::RowWriter,
    service::NamespaceWriteService,
    store::{FactWrite, FactWriter, NamespaceWrite},
};

type Print = fn(&NamespaceName, &Namespace<RelationName>) -> Result<String, String>;

fn print(name: &NamespaceName, relations: &Namespace<RelationName>) -> Result<String, String> {
    namespace::print(name, relations).map_err(|error| error.to_string())
}

struct World {
    table: &'static MemoryTable,
    sequence: &'static MemorySequence,
    writer: RowWriter<
        &'static MemoryTable,
        &'static MemoryTable,
        DevClock,
        NameRows<&'static MemoryTable>,
        Print,
    >,
}

impl World {
    fn new() -> Self {
        let table = Box::leak(Box::new(MemoryTable::new()));
        let sequence = Box::leak(Box::new(MemorySequence::new()));
        let names = NameRows::new(&*table, Consistency::Strong);
        Self {
            table,
            sequence,
            writer: RowWriter::new(&*table, &*table, DevClock::default(), names, print as Print),
        }
    }

    fn names(&self) -> ReadingDictionary<NameRows<&'static MemoryTable>> {
        ReadingDictionary(NameRows::new(self.table, Consistency::Eventual))
    }

    async fn namespace(&self, name: &str, relations: &[&str], write: Write) -> Revision {
        let interner = LeasingInterner::new(
            NameRows::new(self.table, Consistency::Strong),
            NameRowWriter::new(self.table, self.table, self.sequence),
            NonZeroU32::new(8).unwrap(),
        );
        let relations = Namespace::new(
            relations
                .iter()
                .map(|relation| (relation.parse().unwrap(), Rewrite::This))
                .collect(),
        )
        .unwrap();
        let write = match write {
            Write::Create => NamespaceWrite::Create(relations),
            Write::After(based_on) => NamespaceWrite::NewVersion {
                based_on,
                namespace: relations,
            },
            Write::Retire(based_on) => NamespaceWrite::Retire { based_on },
        };
        NamespaceWriteService::new(&interner, &self.writer)
            .write(&name.parse().unwrap(), write)
            .await
            .unwrap()
    }

    async fn fact(&self, name: &FactName) -> Fact {
        resolve::fact(&self.names(), name).await.unwrap()
    }

    async fn add(&self, names: &[FactName]) -> Revision {
        let mut writes = Vec::new();
        for name in names {
            writes.push(FactWrite::Insert(self.fact(name).await));
        }
        self.writer.write(&writes).await.unwrap()
    }

    async fn read(&self, set: &str, subject: Option<SubjectName>) -> (Vec<String>, Revision) {
        let names = self.names();
        let read = FactReadService::new(&names, &self.table)
            .read(&set.parse().unwrap(), subject.as_ref(), None)
            .await
            .unwrap();
        let facts = read.facts.iter().map(ToString::to_string).collect();
        (facts, read.revision)
    }
}

enum Write {
    Create,
    After(Revision),
    Retire(Revision),
}

fn fact(set: &str, subject: SubjectName) -> FactName {
    FactName {
        subjectset: set.parse::<SubjectsetName>().unwrap(),
        subject,
    }
}

fn person(name: &str) -> SubjectName {
    SubjectName::Identity(name.parse().unwrap())
}

#[tokio::test]
async fn facts_read_back_in_subject_order_and_one_by_one() {
    let world = World::new();
    world.namespace("file", &["owner"], Write::Create).await;
    world.namespace("group", &["member"], Write::Create).await;
    let eng = SubjectName::Subjectset("group:eng#member".parse().unwrap());
    let member = SubjectName::Resource("group:eng".parse().unwrap());
    let written = world
        .add(&[
            fact("file:a#owner", person("bob")),
            fact("file:a#owner", member.clone()),
            fact("file:a#owner", eng.clone()),
            fact("file:a#owner", person("alice")),
        ])
        .await;
    let (facts, revision) = world.read("file:a#owner", None).await;
    assert_eq!(
        facts,
        [
            "file:a#owner@alice",
            "file:a#owner@bob",
            "file:a#owner@group:eng#member",
            "file:a#owner@group:eng",
        ]
    );
    assert_eq!(revision, written);
    for (subject, found) in [
        (person("bob"), vec!["file:a#owner@bob"]),
        (eng, vec!["file:a#owner@group:eng#member"]),
        (person("carol"), vec![]),
        (SubjectName::Resource("user:x".parse().unwrap()), vec![]),
    ] {
        assert_eq!(world.read("file:a#owner", Some(subject)).await.0, found);
    }
    for set in ["user:a#owner", "file:a#editor", "file:b#owner"] {
        assert!(world.read(set, None).await.0.is_empty(), "{set}");
    }
}

#[tokio::test]
async fn a_fact_under_a_dropped_relation_counts_for_nothing() {
    let world = World::new();
    let first = world
        .namespace("file", &["owner", "banned"], Write::Create)
        .await;
    world.add(&[fact("file:a#banned", person("bob"))]).await;
    let dropped = world
        .namespace("file", &["owner"], Write::After(first))
        .await;
    assert!(world.read("file:a#banned", None).await.0.is_empty());
    let back = world
        .namespace("file", &["owner", "banned"], Write::After(dropped))
        .await;
    assert!(world.read("file:a#banned", None).await.0.is_empty());
    world.add(&[fact("file:a#banned", person("bob"))]).await;
    assert_eq!(
        world.read("file:a#banned", None).await.0,
        ["file:a#banned@bob"]
    );
    world.namespace("file", &[], Write::Retire(back)).await;
    assert!(world.read("file:a#banned", None).await.0.is_empty());
}

#[tokio::test]
async fn a_namespace_reads_as_its_stored_document() {
    let world = World::new();
    let names = world.names();
    let service = NamespaceReadService::new(&names, &world.table);
    let file = "file".parse().unwrap();
    assert_eq!(service.read(&file, None).await.unwrap().version, None);
    let created = world
        .namespace("file", &["viewer", "owner"], Write::Create)
        .await;
    let read = service.read(&file, None).await.unwrap();
    let version = read.version.unwrap();
    assert_eq!(version.revision, created);
    assert_eq!(
        version.document,
        "{\n  \"file\": {\n    \"owner\": \"this\",\n    \"viewer\": \"this\"\n  }\n}\n"
    );
    world.namespace("file", &[], Write::Retire(created)).await;
    assert_eq!(service.read(&file, None).await.unwrap().version, None);
    let never = "folder".parse().unwrap();
    assert_eq!(service.read(&never, None).await.unwrap().version, None);
}

#[tokio::test]
async fn a_zookie_ahead_of_the_latest_revision_is_refused() {
    let world = World::new();
    let created = world.namespace("file", &["owner"], Write::Create).await;
    let names = world.names();
    let ahead = Some(Revision(created.0 + 1));
    let facts = FactReadService::new(&names, &world.table)
        .read(&"file:a#owner".parse().unwrap(), None, ahead)
        .await;
    assert!(matches!(facts, Err(ReadError::Ahead { .. })), "{facts:?}");
    let namespaces = NamespaceReadService::new(&names, &world.table)
        .read(&"file".parse().unwrap(), ahead)
        .await;
    assert!(
        matches!(namespaces, Err(ReadError::Ahead { .. })),
        "{namespaces:?}"
    );
    let at = NamespaceReadService::new(&names, &world.table)
        .read(&"file".parse().unwrap(), Some(created))
        .await
        .unwrap();
    assert_eq!(at.revision, created);
}
