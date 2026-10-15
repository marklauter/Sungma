//! The in-memory stores run the tests every store runs.

use sungma_memory::{MemoryDictionary, MemoryStore};
use sungma_write::testkit::Stores;

#[derive(Default)]
struct Memory {
    store: MemoryStore,
    names: MemoryDictionary,
}

impl Stores for Memory {
    type Reader = MemoryStore;
    type Writer = MemoryStore;
    type Names = MemoryDictionary;

    fn reader(&self) -> &MemoryStore {
        &self.store
    }

    fn writer(&self) -> &MemoryStore {
        &self.store
    }

    fn names(&self) -> &MemoryDictionary {
        &self.names
    }
}

async fn memory() -> Memory {
    Memory::default()
}

sungma_write::store_tests!(memory, cases = 256, shrink = 1024);
