//! Namespace writes. A write reads the namespace's latest entry and
//! compares its version with the one the write is based on. Its unit of
//! work moves the head, stores the new entry, and makes it the latest, on
//! condition the latest row still holds what the writer read, so of two
//! writes based on one version, one is refused. A version is stored as its
//! printed document with names, beside its epochs.

use sungma::{
    id::NamespaceId, namespace::Namespace, revision::Revision, store::StoreError, version::Epochs,
};
use sungma_clock::Clock;
use sungma_edge::{
    read::{Consistency, ItemReader},
    write::{ItemWriter, Work},
};
use sungma_names::{
    resolve,
    store::{NameReader, ReadingDictionary},
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
        let names = ReadingDictionary(&self.names);
        let name = resolve::namespace_name(&names, namespace).await?;
        let named = resolve::namespace_names(&names, relations).await?;
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
                return Err(NamespaceWriteError::NothingToRetire);
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
        // Every step guards what the write read.
        Ok(self.commit(work, reading, |_| false).await?)
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
