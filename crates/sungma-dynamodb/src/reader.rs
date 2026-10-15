//! The read ports.

use std::sync::Arc;

use sungma::{
    graph::{Subject, Subjectset},
    id::NamespaceId,
    revision::Revision,
    store::{FactReader, NameReader, NamespaceReader, Pool, StoreError},
    version::Version,
};

use crate::{key, table::Table};

/// Reads facts, namespace versions and names. Every read is eventually
/// consistent: a read is a snapshot, stale once it's taken.
#[derive(Clone, Debug)]
pub struct DynamoReader {
    table: Table,
}

impl DynamoReader {
    pub fn new(table: Table) -> Self {
        Self { table }
    }
}

impl FactReader for DynamoReader {
    async fn head(&self) -> Result<Revision, StoreError> {
        self.table.head(false).await
    }

    async fn written(
        &self,
        set: &Subjectset,
        subject: &Subject,
        revision: Revision,
    ) -> Result<Option<Revision>, StoreError> {
        let spans = self
            .table
            .spans(set, Some(key::subject(subject)), revision)
            .await?;
        Ok(spans.first().map(|(_, written)| *written))
    }

    async fn subjectsets(
        &self,
        set: &Subjectset,
        revision: Revision,
    ) -> Result<Vec<(Subjectset, Revision)>, StoreError> {
        let spans = self
            .table
            .spans(set, Some(key::subjectsets()), revision)
            .await?;
        Ok(spans
            .into_iter()
            .filter_map(|(subject, written)| match subject {
                Subject::Subjectset(nested) => Some((nested, written)),
                _ => None,
            })
            .collect())
    }

    async fn subjects(
        &self,
        set: &Subjectset,
        revision: Revision,
    ) -> Result<Vec<(Subject, Revision)>, StoreError> {
        self.table.spans(set, None, revision).await
    }
}

impl NamespaceReader for DynamoReader {
    async fn version(
        &self,
        namespace: NamespaceId,
        revision: Revision,
    ) -> Result<Option<Arc<Version>>, StoreError> {
        self.table.version_at(namespace, revision).await
    }
}

impl NameReader for DynamoReader {
    async fn id(&self, pool: Pool, name: &str) -> Result<Option<u32>, StoreError> {
        self.table.id_of(pool, name, false).await
    }

    async fn name(&self, pool: Pool, id: u32) -> Result<Option<String>, StoreError> {
        self.table.name_of(pool, id).await
    }
}
