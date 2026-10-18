//! The head row: written by its steps, and read back.

use sungma::revision::Revision;
use sungma_schema::head;
use sungma_table::{
    memory::MemoryTable,
    read::Consistency,
    write::{ItemWriter, Work},
};

const READ: Consistency = Consistency::Strong;

#[tokio::test]
async fn the_head_reads_the_revision_its_last_step_moved_it_to() {
    let table = MemoryTable::new();
    assert_eq!(head::read(&table, READ).await.unwrap(), None);
    for (seen, at) in [(None, 5), (Some(Revision(5)), 8)] {
        let step = head::step(seen, Revision(at));
        table.commit(Work(vec![step])).await.unwrap();
        assert_eq!(head::read(&table, READ).await.unwrap(), Some(Revision(at)));
    }
}

/// A step from a head the writer didn't read is refused.
#[tokio::test]
async fn a_step_from_a_stale_head_is_refused() {
    let table = MemoryTable::new();
    let first = head::step(None, Revision(5));
    table.commit(Work(vec![first])).await.unwrap();
    for seen in [None, Some(Revision(4))] {
        let stale = head::step(seen, Revision(9));
        assert!(table.commit(Work(vec![stale])).await.is_err());
    }
    assert_eq!(head::read(&table, READ).await.unwrap(), Some(Revision(5)));
}
