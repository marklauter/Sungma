//! Namespace writes. A write reads the namespace's latest entry and
//! compares its version with the one the write is based on. Its unit of
//! work moves the head, stores the new entry, and makes it the latest, on
//! condition the latest row still holds what the writer read, so of two
//! writes based on one version, one is refused. A version is stored as its
//! printed document with names, beside its epochs.

use std::collections::HashMap;

use sungma::{
    id::NamespaceId,
    name::RelationName,
    namespace::{Namespace, NamespaceError},
    revision::Revision,
    store::{NameReader, Pool, StoreError},
    version::{Epochs, HistoryError},
};
use sungma_clock::Clock;
use sungma_edge::{
    read::{Consistency, ItemReader},
    write::{ItemWriter, Work},
};
use sungma_schema::{
    head,
    namespaces::{self, Entry},
};

use super::{Failure, Printer, RowWriter, Tried, gave_up};
use crate::store::{NamespaceWrite, NamespaceWriteError, NamespaceWriter};

impl From<Failure> for NamespaceWriteError {
    fn from(failure: Failure) -> Self {
        match failure {
            Failure::Store(error) => error.into(),
            Failure::Clock(fault) => fault.into(),
        }
    }
}

/// A rename that failed as a store does.
struct Unnamed(StoreError);

impl From<NamespaceError> for Unnamed {
    fn from(error: NamespaceError) -> Self {
        Self(StoreError(error.to_string()))
    }
}

impl<R, W, C, N, P> RowWriter<R, W, C, N, P>
where
    R: ItemReader + Sync,
    W: ItemWriter + Sync,
    C: Clock + Sync,
    N: NameReader + Sync,
    P: Printer + Sync,
{
    /// The version printed with its names. Names are stored before the
    /// versions that use them, so one that's missing has failed.
    async fn document(
        &self,
        namespace: NamespaceId,
        relations: &Namespace,
    ) -> Result<String, StoreError> {
        let unnamed = |what: String| StoreError(format!("{what} has no interned name"));
        let name = self
            .names
            .name(Pool::Namespaces, namespace.0)
            .await?
            .ok_or_else(|| unnamed(format!("namespace {namespace}")))?;
        let name = name
            .parse()
            .map_err(|_| StoreError(format!("namespace {namespace} is named '{name}'")))?;
        let mut names = HashMap::new();
        for relation in relations.names() {
            let named = self
                .names
                .name(Pool::Relations, relation.0)
                .await?
                .ok_or_else(|| unnamed(format!("relation {relation}")))?;
            names.insert(*relation, RelationName::new_unchecked(named));
        }
        let named = relations
            .try_map(|relation| {
                names
                    .get(relation)
                    .cloned()
                    .ok_or_else(|| Unnamed(unnamed(format!("relation {relation}"))))
            })
            .map_err(|Unnamed(error)| error)?;
        self.printer.print(&name, &named).map_err(StoreError)
    }

    async fn try_write_namespace(
        &self,
        namespace: NamespaceId,
        write: &NamespaceWrite,
    ) -> Result<Tried, NamespaceWriteError> {
        let strong = Consistency::Strong;
        let seen_head = head::read(&self.reader, strong).await?;
        let seen = namespaces::latest(&self.reader, namespace, strong).await?;
        let previous = seen.as_ref().and_then(Entry::epochs);
        let version = seen
            .as_ref()
            .filter(|entry| entry.epochs().is_some())
            .map(Entry::revision);
        let (expected, next) = match write {
            NamespaceWrite::Create(next) => (None, Some(next)),
            NamespaceWrite::NewVersion {
                based_on,
                namespace,
            } => (Some(*based_on), Some(namespace)),
            NamespaceWrite::Retire { .. } if version.is_none() => {
                return Err(HistoryError::NothingToRetire.into());
            }
            NamespaceWrite::Retire { based_on } => (Some(*based_on), None),
        };
        if version != expected {
            return Err(NamespaceWriteError::Conflict { latest: version });
        }
        let document = match next {
            Some(next) => Some(self.document(namespace, next).await?),
            None => None,
        };
        let Some(reading) = self.stamp(seen_head).await? else {
            return Ok(Tried::Lost);
        };
        let at = reading.revision;
        let entry = match (next, document) {
            (Some(next), Some(document)) => Entry::Version {
                revision: at,
                document,
                epochs: Epochs::following(
                    previous,
                    next.relations().map(|(relation, _)| *relation),
                    at,
                ),
            },
            _ => Entry::Retired { revision: at },
        };
        let mut work = Work::new();
        work.push(head::step(seen_head, at));
        for step in namespaces::written(namespace, seen.as_ref(), &entry) {
            work.push(step);
        }
        Ok(self.commit(work, reading).await?)
    }
}

impl<R, W, C, N, P> NamespaceWriter for RowWriter<R, W, C, N, P>
where
    R: ItemReader + Sync,
    W: ItemWriter + Sync,
    C: Clock + Sync,
    N: NameReader + Sync,
    P: Printer + Sync,
{
    async fn write_namespace(
        &self,
        namespace: NamespaceId,
        write: NamespaceWrite,
    ) -> Result<Revision, NamespaceWriteError> {
        let _writing = self.lock.lock().await;
        for _ in 0..super::ATTEMPTS {
            if let Tried::Written(reading) = self.try_write_namespace(namespace, &write).await? {
                return Ok(self.settle(reading).await?);
            }
        }
        Err(gave_up().into())
    }
}
