//! Names over the memory table.

use std::{
    num::NonZeroU32,
    sync::atomic::{AtomicUsize, Ordering},
};

use sungma_names::store::{
    NameReader,
    Pool::{Namespaces, Relations},
};
use sungma_names::{intern::LeasingInterner, store::Interner, store::NameWriter};
use sungma_names::{read::NameRows, write::NameRowWriter};
use sungma_table::{
    TableError,
    item::{Item, Value},
    key::Key,
    memory::{MemorySequence, MemoryTable},
    read::Consistency,
    sequence::Sequence,
    write::{CommitError, ItemWriter, Step, Work},
};

fn reader(table: &MemoryTable) -> NameRows<&MemoryTable> {
    NameRows::new(table, Consistency::Eventual)
}

fn writer<'a>(
    table: &'a MemoryTable,
    sequence: &'a MemorySequence,
) -> NameRowWriter<&'a MemoryTable, &'a MemoryTable, &'a MemorySequence> {
    NameRowWriter::new(table, table, sequence)
}

#[tokio::test]
async fn a_name_is_stored_once_and_read_both_ways() {
    let (table, sequence) = (MemoryTable::new(), MemorySequence::new());
    let writer = writer(&table, &sequence);
    assert_eq!(
        writer
            .insert_if_absent(Relations, "owner", 7)
            .await
            .unwrap(),
        7
    );
    // Stored again under another id, it keeps the first.
    assert_eq!(
        writer
            .insert_if_absent(Relations, "owner", 8)
            .await
            .unwrap(),
        7
    );
    let names = reader(&table);
    assert_eq!(names.id(Relations, "owner").await.unwrap(), Some(7));
    assert_eq!(
        names.name(Relations, 7).await.unwrap().as_deref(),
        Some("owner")
    );
    assert_eq!(names.name(Relations, 8).await.unwrap(), None);
    // Pools are apart.
    assert_eq!(names.id(Namespaces, "owner").await.unwrap(), None);
    assert_eq!(names.name(Namespaces, 7).await.unwrap(), None);
}

#[tokio::test]
async fn an_id_another_name_holds_is_refused() {
    let (table, sequence) = (MemoryTable::new(), MemorySequence::new());
    let writer = writer(&table, &sequence);
    writer
        .insert_if_absent(Relations, "owner", 7)
        .await
        .unwrap();
    let taken = writer.insert_if_absent(Relations, "viewer", 7).await;
    assert!(taken.unwrap_err().0.contains("taken"));
    assert_eq!(reader(&table).id(Relations, "viewer").await.unwrap(), None);
}

#[tokio::test]
async fn a_lease_takes_what_is_left_of_the_ids() {
    let (table, sequence) = (MemoryTable::new(), MemorySequence::new());
    let writer = writer(&table, &sequence);
    assert_eq!(writer.lease(Relations, 3).await.unwrap(), 0..3);
    assert_eq!(writer.lease(Namespaces, 2).await.unwrap(), 0..2);
    let counter = Key::new((3u8, 1u8), 0u8);
    sequence
        .next(&counter, u64::from(u32::MAX) - 5)
        .await
        .unwrap();
    assert_eq!(
        writer.lease(Relations, 10).await.unwrap(),
        u32::MAX - 2..u32::MAX
    );
    assert!(writer.lease(Relations, 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn the_interner_mints_through_the_rows() {
    let (table, sequence) = (MemoryTable::new(), MemorySequence::new());
    let block = NonZeroU32::new(4).unwrap();
    let a = LeasingInterner::new(reader(&table), writer(&table, &sequence), block);
    let b = LeasingInterner::new(reader(&table), writer(&table, &sequence), block);
    let owner = a.intern(Relations, "owner").await.unwrap();
    assert_eq!(b.intern(Relations, "owner").await.unwrap(), owner);
    let viewer = b.intern(Relations, "viewer").await.unwrap();
    assert_ne!(viewer, owner);
    assert_eq!(a.intern(Relations, "viewer").await.unwrap(), viewer);
}

#[tokio::test]
async fn a_row_sungma_did_not_write_is_refused() {
    let table = MemoryTable::new();
    let rows = [
        (
            Key::new((3u8, 1u8), (1u8, "owner")),
            Item::new().with("id", Value::Text("7".to_owned())),
        ),
        (Key::new((3u8, 1u8), (1u8, "editor")), Item::new()),
        (
            Key::new((3u8, 1u8), (1u8, "big")),
            Item::new().with("id", Value::Number(u64::MAX)),
        ),
        (
            Key::new((3u8, 1u8), (2u8, 7u32)),
            Item::new().with("name", Value::Number(7)),
        ),
    ];
    for (key, item) in rows {
        table
            .commit(Work(vec![Step::Insert { key, item }]))
            .await
            .unwrap();
    }
    let names = reader(&table);
    for name in ["owner", "editor", "big"] {
        assert!(names.id(Relations, name).await.is_err(), "{name}");
    }
    assert!(names.name(Relations, 7).await.is_err());
}

/// A writer that answers each commit with the next of `answers`, then
/// commits through the table.
struct Scripted<'a> {
    table: &'a MemoryTable,
    answers: Vec<CommitError>,
    calls: AtomicUsize,
}

impl ItemWriter for Scripted<'_> {
    fn max_steps(&self) -> usize {
        self.table.max_steps()
    }

    async fn commit(&self, work: Work) -> Result<(), CommitError> {
        let call = self.calls.fetch_add(1, Ordering::Relaxed);
        match self.answers.get(call) {
            Some(answer) => Err(answer.clone()),
            None => self.table.commit(work).await,
        }
    }
}

#[tokio::test]
async fn an_insert_tries_again_after_other_work_in_flight() {
    let (table, sequence) = (MemoryTable::new(), MemorySequence::new());
    let scripted = Scripted {
        table: &table,
        answers: vec![CommitError::Contended, CommitError::Contended],
        calls: AtomicUsize::new(0),
    };
    let writer = NameRowWriter::new(&table, scripted, &sequence);
    assert_eq!(
        writer
            .insert_if_absent(Relations, "owner", 1)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        reader(&table).id(Relations, "owner").await.unwrap(),
        Some(1)
    );
}

#[tokio::test]
async fn an_insert_that_cannot_commit_fails() {
    let (table, sequence) = (MemoryTable::new(), MemorySequence::new());
    let answers = [
        vec![CommitError::Contended; 16],
        vec![CommitError::Unwritable("too big".to_owned())],
        vec![CommitError::Failed(TableError("down".to_owned()))],
    ];
    for answers in answers {
        let scripted = Scripted {
            table: &table,
            answers,
            calls: AtomicUsize::new(0),
        };
        let writer = NameRowWriter::new(&table, scripted, &sequence);
        assert!(
            writer
                .insert_if_absent(Relations, "owner", 1)
                .await
                .is_err()
        );
    }
    assert_eq!(reader(&table).id(Relations, "owner").await.unwrap(), None);
}
