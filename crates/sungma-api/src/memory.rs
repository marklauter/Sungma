//! The in-memory store as one reader and writer: names in a
//! [`MemoryDictionary`], facts and namespace versions in a [`MemoryStore`].

use std::{ops::Range, sync::Arc};

use sungma::{
    graph::{Subject, Subjectset},
    id::NamespaceId,
    revision::Revision,
    store::{FactReader, NameReader, NamespaceReader, Pool, StoreError},
    version::Version,
};
use sungma_memory::{MemoryDictionary, MemoryStore};
use sungma_write::store::{
    FactWrite, FactWriteError, FactWriter, NameWriter, NamespaceWrite, NamespaceWriteError,
    NamespaceWriter,
};

#[derive(Debug, Default)]
pub struct Memory {
    pub names: MemoryDictionary,
    pub store: MemoryStore,
}

impl NameReader for Memory {
    async fn id(&self, pool: Pool, name: &str) -> Result<Option<u32>, StoreError> {
        NameReader::id(&self.names, pool, name).await
    }

    async fn name(&self, pool: Pool, id: u32) -> Result<Option<String>, StoreError> {
        NameReader::name(&self.names, pool, id).await
    }
}

impl NameWriter for Memory {
    async fn insert_if_absent(&self, pool: Pool, name: &str, id: u32) -> Result<u32, StoreError> {
        self.names.insert_if_absent(pool, name, id).await
    }

    async fn lease(&self, pool: Pool, count: u32) -> Result<Range<u32>, StoreError> {
        self.names.lease(pool, count).await
    }
}

impl FactReader for Memory {
    async fn head(&self) -> Result<Revision, StoreError> {
        self.store.head().await
    }

    async fn written(
        &self,
        set: &Subjectset,
        subject: &Subject,
        revision: Revision,
    ) -> Result<Option<Revision>, StoreError> {
        self.store.written(set, subject, revision).await
    }

    async fn subjectsets(
        &self,
        set: &Subjectset,
        revision: Revision,
    ) -> Result<Vec<(Subjectset, Revision)>, StoreError> {
        self.store.subjectsets(set, revision).await
    }

    async fn subjects(
        &self,
        set: &Subjectset,
        revision: Revision,
    ) -> Result<Vec<(Subject, Revision)>, StoreError> {
        self.store.subjects(set, revision).await
    }
}

impl FactWriter for Memory {
    async fn write(&self, writes: &[FactWrite]) -> Result<Revision, FactWriteError> {
        self.store.write(writes).await
    }
}

impl NamespaceReader for Memory {
    async fn version(
        &self,
        namespace: NamespaceId,
        revision: Revision,
    ) -> Result<Option<Arc<Version>>, StoreError> {
        self.store.version(namespace, revision).await
    }
}

impl NamespaceWriter for Memory {
    async fn write_namespace(
        &self,
        namespace: NamespaceId,
        write: NamespaceWrite,
    ) -> Result<Revision, NamespaceWriteError> {
        self.store.write_namespace(namespace, write).await
    }
}
