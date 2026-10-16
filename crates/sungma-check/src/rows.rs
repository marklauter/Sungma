//! Check's reads over the table: facts from their rows, and namespace
//! versions built from their rows and kept in a buffer. A stored version
//! never changes, so a buffered one is never stale: a read finds which
//! entry covers its revision, takes the version from the buffer, and reads
//! through, parsing the document and resolving its names, only on a miss.
//! The buffer evicts the oldest versions once it's full.

use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

use crate::{store::FactReader, store::NamespaceReader};
use parking_lot::Mutex;
use sungma::{
    graph::{Subject, Subjectset},
    id::{NamespaceId, RelationId},
    name::{NamespaceName, RelationName},
    namespace::{Namespace, NamespaceError},
    revision::Revision,
    store::StoreError,
    version::Version,
};
use sungma_names::read::NameRows;
use sungma_names::{store::NameReader, store::Pool};
use sungma_schema::{
    facts::{self, Rows},
    head,
    namespaces::{self, Entry},
};
use sungma_table::read::{Consistency, ItemReader};

/// Reads a stored namespace document back as a namespace with names.
/// Injected, so Check needn't know the document language.
pub trait Parser {
    fn parse(&self, document: &str) -> Result<(NamespaceName, Namespace<RelationName>), String>;
}

impl<F> Parser for F
where
    F: Fn(&str) -> Result<(NamespaceName, Namespace<RelationName>), String>,
{
    fn parse(&self, document: &str) -> Result<(NamespaceName, Namespace<RelationName>), String> {
        self(document)
    }
}

/// Versions by namespace and the revision each opened at, oldest first.
#[derive(Debug, Default)]
struct Buffer {
    versions: HashMap<(NamespaceId, Revision), Arc<Version>>,
    order: VecDeque<(NamespaceId, Revision)>,
}

impl Buffer {
    fn get(&self, key: (NamespaceId, Revision)) -> Option<Arc<Version>> {
        self.versions.get(&key).cloned()
    }

    fn put(&mut self, key: (NamespaceId, Revision), version: Arc<Version>, capacity: usize) {
        if self.versions.insert(key, version).is_some() {
            return;
        }
        self.order.push_back(key);
        while self.order.len() > capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.versions.remove(&oldest);
            }
        }
    }
}

/// Reads facts and namespace versions from rows through `items`, finding
/// relation ids through `names`, with documents read by `parser`. Every
/// read is eventual: a read is a snapshot, stale once it's taken.
#[derive(Debug)]
pub struct RowReader<R, N, P> {
    items: R,
    names: N,
    parser: P,
    capacity: usize,
    buffer: Mutex<Buffer>,
}

impl<R, N, P> RowReader<R, N, P> {
    /// Buffers at most `capacity` versions.
    pub fn new(items: R, names: N, parser: P, capacity: usize) -> Self {
        Self {
            items,
            names,
            parser,
            capacity,
            buffer: Mutex::default(),
        }
    }
}

const READ: Consistency = Consistency::Eventual;

impl<R: ItemReader + Sync, N: Sync, P: Sync> FactReader for RowReader<R, N, P> {
    async fn head(&self) -> Result<Revision, StoreError> {
        Ok(head::read(&self.items, READ).await?.unwrap_or(Revision(0)))
    }

    async fn written(
        &self,
        set: &Subjectset,
        subject: &Subject,
        revision: Revision,
    ) -> Result<Option<Revision>, StoreError> {
        let rows = facts::rows(&self.items, set, Rows::Of(subject), READ).await?;
        Ok(facts::covering(&rows, revision)
            .into_iter()
            .find(|(of, _)| of == subject)
            .map(|(_, written)| written))
    }

    async fn subjectsets(
        &self,
        set: &Subjectset,
        revision: Revision,
    ) -> Result<Vec<(Subjectset, Revision)>, StoreError> {
        let rows = facts::rows(&self.items, set, Rows::Subjectsets, READ).await?;
        Ok(facts::covering(&rows, revision)
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
        let rows = facts::rows(&self.items, set, Rows::All, READ).await?;
        Ok(facts::covering(&rows, revision))
    }
}

/// A rename that failed as a store does.
struct Unknown(StoreError);

impl From<NamespaceError> for Unknown {
    fn from(error: NamespaceError) -> Self {
        Self(StoreError(error.to_string()))
    }
}

impl<R, N, P> RowReader<R, N, P>
where
    R: ItemReader + Sync,
    N: NameReader + Sync,
    P: Parser + Sync,
{
    /// The version a stored entry holds, read through: its document parsed
    /// and its names resolved to ids. Names are stored before the versions
    /// that use them, so a name that's missing has failed.
    async fn version(
        &self,
        revision: Revision,
        document: &str,
        epochs: sungma::version::Epochs,
    ) -> Result<Version, StoreError> {
        let (_, relations) = self
            .parser
            .parse(document)
            .map_err(|error| StoreError(format!("a stored document is refused: {error}")))?;
        let mut ids = HashMap::new();
        for name in relations.names() {
            let mut id = self.names.id(Pool::Relations, name.as_str()).await?;
            // Names are stored before the versions that use them, so a name
            // an eventual read misses is read again strongly.
            if id.is_none() {
                let strong = NameRows::new(&self.items, Consistency::Strong);
                id = strong.id(Pool::Relations, name.as_str()).await?;
            }
            let id = id.ok_or_else(|| StoreError(format!("relation '{name}' isn't interned")))?;
            ids.insert(name.clone(), RelationId(id));
        }
        let namespace = relations
            .try_map(|name| {
                ids.get(name)
                    .copied()
                    .ok_or_else(|| Unknown(StoreError(format!("relation '{name}' isn't interned"))))
            })
            .map_err(|Unknown(error)| error)?;
        Version::stored(namespace, revision, epochs).map_err(|error| StoreError(error.to_string()))
    }
}

impl<R, N, P> NamespaceReader for RowReader<R, N, P>
where
    R: ItemReader + Sync,
    N: NameReader + Sync,
    P: Parser + Sync,
{
    async fn version(
        &self,
        namespace: NamespaceId,
        revision: Revision,
    ) -> Result<Option<Arc<Version>>, StoreError> {
        let Some(entry) = namespaces::at(&self.items, namespace, revision, READ).await? else {
            return Ok(None);
        };
        let Entry::Version {
            revision: opened,
            document,
            epochs,
        } = entry
        else {
            return Ok(None);
        };
        let key = (namespace, opened);
        if let Some(version) = self.buffer.lock().get(key) {
            return Ok(Some(version));
        }
        let version = Arc::new(self.version(opened, &document, epochs).await?);
        self.buffer
            .lock()
            .put(key, Arc::clone(&version), self.capacity);
        Ok(Some(version))
    }
}
