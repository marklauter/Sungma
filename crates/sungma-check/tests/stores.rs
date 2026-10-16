//! The write and Check contexts over the memory edge run the tests every
//! store runs.

mod common;

use common::Contexts;
use sungma_edge::memory::{MemorySequence, MemoryTable};

async fn memory() -> Contexts<&'static MemoryTable, &'static MemorySequence> {
    let table = Box::leak(Box::new(MemoryTable::new()));
    let sequence = Box::leak(Box::new(MemorySequence::new()));
    Contexts::new(&*table, &*sequence).await
}

sungma_write::store_tests!(memory, cases = 256, shrink = 1024);
