//! What Check's reader does over a table beyond the shared store tests:
//! rows Sungma didn't write, names an eventual read misses, and the
//! versions it keeps parsed.

use std::sync::Arc;

use sungma::{id::NamespaceId, revision::Revision};
use sungma_check::{
    rows::RowReader,
    store::{FactReader, NamespaceReader},
};
use sungma_clock::DevClock;
use sungma_names::read::NameRows;
use sungma_names::store::Pool;
use sungma_table::{
    item::{Item, Value},
    key::{Key, encode},
    read::Consistency,
    write::{Step, Work},
};
use sungma_write::{
    store::{NamespaceWrite, NamespaceWriter},
    testkit::{
        Parse, parse,
        rows::{DOC, Memory, OWNER, Table, declared, declaring, owner, reader, writer},
    },
};

fn memory() -> Memory {
    Memory::default()
}

/// Rows Sungma didn't write are refused when read.
async fn a_row_sungma_did_not_write_is_refused<E: Table>(table: E) {
    declared(&table).await;
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
    let reader = reader(&table);
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
        table.commit(insert(key, item)).await.unwrap();
    }
    assert!(reader.subjects(&set, Revision(10)).await.is_err());
    let other = owner("b", "x").subjectset;
    assert!(reader.subjects(&other, Revision(10)).await.is_err());
    // A latest row without a revision.
    let ghost = NamespaceId(9);
    table
        .commit(insert(
            sungma_schema::namespaces::latest_key(ghost),
            Item::new().with("document", Value::Text("{}".to_owned())),
        ))
        .await
        .unwrap();
    assert!(reader.version(ghost, Revision(10)).await.is_err());
}

#[tokio::test]
async fn a_row_sungma_did_not_write_is_refused_on_the_memory_table() {
    a_row_sungma_did_not_write_is_refused(memory()).await;
}

/// A name reader that lags: it never finds a name by key, as an eventual
/// read can miss a name stored moments ago.
#[derive(Clone)]
struct Lagging<E>(NameRows<E>);

impl<E: Table> sungma_names::store::NameReader for Lagging<E> {
    async fn id(&self, _: Pool, _: &str) -> Result<Option<u32>, sungma::store::StoreError> {
        Ok(None)
    }

    async fn name(&self, pool: Pool, id: u32) -> Result<Option<String>, sungma::store::StoreError> {
        self.0.name(pool, id).await
    }
}

/// A version read whose eventual name read misses a name reads it again
/// strongly, since names are stored before the versions that use them.
#[tokio::test]
async fn a_version_read_that_misses_a_name_reads_it_again() {
    let table = memory();
    declared(&table).await;
    let lagging = Lagging(NameRows::new(table.clone(), Consistency::Eventual));
    let reader = RowReader::new(table.clone(), lagging, parse as Parse, 16);
    let version = reader.version(DOC, Revision(1)).await;
    assert!(matches!(version, Ok(Some(_))), "{version:?}");
}

/// Check's reader keeps the versions it parsed, up to its capacity, and
/// lets the oldest go first.
#[tokio::test]
async fn the_reader_keeps_parsed_versions_up_to_its_capacity() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let table = memory();
    let first = declared(&table).await;
    let writer = writer(&table, DevClock::default());
    let mut revisions = vec![first];
    for _ in 0..2 {
        let next = NamespaceWrite::NewVersion {
            based_on: *revisions.last().unwrap(),
            namespace: declaring(OWNER),
        };
        revisions.push(writer.write_namespace(DOC, next).await.unwrap());
    }
    let parsed = Arc::new(AtomicUsize::new(0));
    let counting = {
        let parsed = Arc::clone(&parsed);
        move |document: &str| {
            parsed.fetch_add(1, Ordering::Relaxed);
            parse(document)
        }
    };
    let reader = RowReader::new(
        table.clone(),
        NameRows::new(table.clone(), Consistency::Eventual),
        counting,
        2,
    );
    // The third version lets the first go, and the first is parsed again.
    for (at, parses) in [(0, 1), (1, 2), (0, 2), (2, 3), (1, 3), (0, 4)] {
        let version = reader.version(DOC, revisions[at]).await.unwrap();
        assert!(version.is_some(), "reading version {at}");
        assert_eq!(
            parsed.load(Ordering::Relaxed),
            parses,
            "reading version {at}"
        );
    }
}
