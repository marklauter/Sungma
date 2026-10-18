//! Namespace rows: entries written by their steps, and read back at a
//! revision.

use std::collections::HashMap;

use sungma::{
    id::{NamespaceId, RelationId},
    revision::Revision,
    version::Epochs,
};
use sungma_schema::namespaces::{self, Entry};
use sungma_table::{
    memory::MemoryTable,
    read::Consistency,
    write::{ItemWriter, Work},
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
    let retired = Entry::Retired {
        revision: Revision(4),
    };
    assert_eq!(retired.epochs(), None);
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
