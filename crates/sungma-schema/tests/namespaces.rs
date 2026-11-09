//! Namespace rows: entries written by their steps, and read back at a
//! revision.

use std::collections::{BTreeMap, HashMap};

use sungma::{
    id::{NamespaceId, RelationId},
    namespace::Constraint,
    revision::Revision,
    version::{Constraints, Epochs},
};
use sungma_schema::namespaces::{self, Entry};
use sungma_table::{
    item::{Item, Value},
    memory::MemoryTable,
    read::Consistency,
    write::{ItemWriter, Step, Work},
};

const READ: Consistency = Consistency::Strong;
const FILE: NamespaceId = NamespaceId(1);
const FOLDER: NamespaceId = NamespaceId(2);

fn version(at: u64, document: &str) -> Entry {
    Entry::Version {
        revision: Revision(at),
        document: document.to_owned(),
        epochs: Epochs::new(HashMap::from([
            (RelationId(1), Revision(1)),
            (RelationId(2), Revision(at)),
        ])),
        constraints: Constraints::new(HashMap::from([
            (
                RelationId(1),
                vec![
                    Constraint::Identity,
                    Constraint::Resource(FOLDER),
                    Constraint::Subjectset(FOLDER, RelationId(2)),
                ],
            ),
            (RelationId(2), vec![Constraint::Identity]),
        ])),
    }
}

async fn write(table: &MemoryTable, namespace: NamespaceId, seen: Option<&Entry>, entry: &Entry) {
    let steps = namespaces::written(namespace, seen, entry);
    table.commit(Work(steps.to_vec())).await.unwrap();
}

#[test]
fn a_version_has_epochs_and_a_retirement_has_none() {
    let entry = version(3, "file");
    let Entry::Version { epochs, .. } = &entry else {
        unreachable!()
    };
    assert_eq!(entry.epochs(), Some(epochs));
    let Entry::Version { constraints, .. } = &entry else {
        unreachable!()
    };
    assert_eq!(entry.constraints(), Some(constraints));
    let retired = Entry::Retired {
        revision: Revision(4),
    };
    assert_eq!(retired.epochs(), None);
    assert_eq!(retired.constraints(), None);
}

/// A latest row holding `constraints` as its constraints, beside a valid
/// revision, document and epochs.
async fn row_with(constraints: Value) -> Result<Option<Entry>, sungma::store::StoreError> {
    let table = MemoryTable::new();
    let item = Item::new()
        .with("revision", Value::Number(3))
        .with("document", Value::Text("file".to_owned()))
        .with("epochs", Value::Map(BTreeMap::new()))
        .with("constraints", constraints);
    let key = namespaces::latest_key(FILE);
    table
        .commit(Work(vec![Step::Insert { key, item }]))
        .await
        .unwrap();
    namespaces::latest(&table, FILE, READ).await
}

fn set(constraints: &[&str]) -> Value {
    Value::Map(
        constraints
            .iter()
            .map(|constraint| ((*constraint).to_owned(), Value::Flag(true)))
            .collect(),
    )
}

#[tokio::test]
async fn constraints_read_back_as_written_with_ids() {
    let constraints = Value::Map(BTreeMap::from([(
        "4".to_owned(),
        set(&["identity", "2", "2#5"]),
    )]));
    let entry = row_with(constraints).await.unwrap().unwrap();
    let admitted = entry.constraints().unwrap();
    let listed: Vec<_> = admitted.iter().collect();
    assert_eq!(
        listed,
        [(
            RelationId(4),
            &[
                Constraint::Identity,
                Constraint::Resource(FOLDER),
                Constraint::Subjectset(FOLDER, RelationId(5)),
            ][..]
        )]
    );
}

#[tokio::test]
async fn a_constraints_row_sungma_did_not_write_is_a_store_failure() {
    for constraints in [
        Value::Number(1),
        Value::Map(BTreeMap::from([("x".to_owned(), set(&["identity"]))])),
        Value::Map(BTreeMap::from([("4".to_owned(), Value::Number(1))])),
        Value::Map(BTreeMap::from([("4".to_owned(), set(&["folder"]))])),
        Value::Map(BTreeMap::from([("4".to_owned(), set(&["2#x"]))])),
        Value::Map(BTreeMap::from([("4".to_owned(), set(&["x#2"]))])),
    ] {
        let read = row_with(constraints.clone()).await;
        assert!(read.is_err(), "{constraints:?}: {read:?}");
    }
}

#[tokio::test]
async fn each_entry_reads_back_as_the_latest() {
    let table = MemoryTable::new();
    assert_eq!(namespaces::latest(&table, FILE, READ).await.unwrap(), None);
    let first = version(3, "file v1");
    write(&table, FILE, None, &first).await;
    assert_eq!(
        namespaces::latest(&table, FILE, READ).await.unwrap(),
        Some(first.clone())
    );
    let retired = Entry::Retired {
        revision: Revision(5),
    };
    write(&table, FILE, Some(&first), &retired).await;
    assert_eq!(
        namespaces::latest(&table, FILE, READ).await.unwrap(),
        Some(retired)
    );
}

#[tokio::test]
async fn each_namespace_keeps_its_own_entries() {
    let table = MemoryTable::new();
    write(&table, FILE, None, &version(3, "file")).await;
    assert_eq!(
        namespaces::latest(&table, FOLDER, READ).await.unwrap(),
        None
    );
    let read = namespaces::at(&table, FOLDER, Revision(9), READ).await;
    assert_eq!(read.unwrap(), None);
}

/// A read at a revision finds the entry whose span covers it: none before
/// the first, the latest from where it opens, and an older one before
/// that.
#[tokio::test]
async fn a_read_at_a_revision_finds_the_entry_that_covers_it() {
    let table = MemoryTable::new();
    let (first, second) = (version(3, "file v1"), version(7, "file v2"));
    write(&table, FILE, None, &first).await;
    write(&table, FILE, Some(&first), &second).await;
    for (at, covering) in [
        (2, None),
        (3, Some(&first)),
        (5, Some(&first)),
        (7, Some(&second)),
        (9, Some(&second)),
    ] {
        let read = namespaces::at(&table, FILE, Revision(at), READ).await;
        assert_eq!(read.unwrap().as_ref(), covering, "at {at}");
    }
}

/// A write based on an entry that isn't the latest is refused.
#[tokio::test]
async fn a_write_from_a_stale_latest_is_refused() {
    let table = MemoryTable::new();
    let (first, second) = (version(3, "file v1"), version(7, "file v2"));
    write(&table, FILE, None, &first).await;
    write(&table, FILE, Some(&first), &second).await;
    let stale = namespaces::written(FILE, Some(&first), &version(9, "file v3"));
    assert!(table.commit(Work(stale.to_vec())).await.is_err());
    let unchanged = namespaces::latest_unchanged(FILE, Some(&first));
    assert!(table.commit(Work(vec![unchanged])).await.is_err());
    let unchanged = namespaces::latest_unchanged(FILE, Some(&second));
    assert!(table.commit(Work(vec![unchanged])).await.is_ok());
}
