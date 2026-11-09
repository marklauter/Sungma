//! Namespace writes. A write reads the namespace's latest entry and
//! compares its version with the one the write is based on. Its unit of
//! work moves the head, stores the new entry, and makes it the latest, on
//! condition the latest row still holds what the writer read, so of two
//! writes based on one version, one is refused. A version is stored as its
//! printed document with names, beside its epochs and its constraints.
//!
//! A new version is checked against the latest versions of the namespaces
//! its constraints name, as [`references`] says, and the unit of work also
//! checks each of those latest rows, so a version is never stored beside a
//! change to a namespace it was judged by.

use std::collections::HashMap;

use sungma::{
    id::{NamespaceId, RelationId},
    name::{NamespaceName, RelationName},
    namespace::Namespace,
    revision::Revision,
    store::StoreError,
    version::{Constraints, Epochs},
};
use sungma_clock::Clock;
use sungma_names::{
    resolve,
    store::{NameReader, ReadingDictionary},
};
use sungma_schema::{
    head,
    namespaces::{self, Entry},
};
use sungma_table::{
    read::{Consistency, ItemReader},
    write::{ItemWriter, Work},
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
    /// The version with its names, and the namespace's name. Names are
    /// stored before the versions that use them, so one that's missing has
    /// failed.
    async fn named(
        &self,
        namespace: NamespaceId,
        relations: &Namespace,
    ) -> Result<(NamespaceName, Namespace<RelationName, NamespaceName>), StoreError> {
        let names = ReadingDictionary(&self.names);
        let name = resolve::namespace_name(&names, namespace).await?;
        let named = resolve::namespace_names(&names, relations).await?;
        Ok((name, named))
    }

    /// The latest entry of each other namespace `relations`' constraints
    /// name, read strongly, `None` for one never written.
    async fn referenced(
        &self,
        namespace: NamespaceId,
        relations: &Namespace,
    ) -> Result<Vec<(NamespaceId, Option<Entry>)>, StoreError> {
        let mut latest = Vec::new();
        for &other in relations.namespaces() {
            if other != namespace {
                let entry = namespaces::latest(&self.reader, other, Consistency::Strong).await?;
                latest.push((other, entry));
            }
        }
        Ok(latest)
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
        let (document, referenced) = match next {
            Some(next) => {
                let (name, named) = self.named(namespace, next).await?;
                let referenced = self.referenced(namespace, next).await?;
                references(&name, &named, next, &referenced)?;
                let document = self.printer.print(&name, &named).map_err(StoreError)?;
                (Some(document), referenced)
            }
            None => (None, Vec::new()),
        };
        let rows = 3 + referenced.len();
        let most = self.items.max_steps();
        if rows > most {
            return Err(NamespaceWriteError::TooLarge { rows, most });
        }
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
                constraints: Constraints::of(next),
            },
            _ => Entry::Retired { revision: at },
        };
        let mut work = Work::new();
        work.push(head::step(seen_head, at));
        for step in namespaces::written(namespace, seen.as_ref(), &entry) {
            work.push(step);
        }
        for (other, latest) in &referenced {
            work.push(namespaces::latest_unchanged(*other, latest.as_ref()));
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

/// Refuses a version that names a relation another namespace doesn't
/// declare, through a constraint `namespace#relation` or the relation a
/// fact-to-subjectset follows, as `docs/specs/namespace-documents.md` says.
/// `named` is the version with names, `ids` the same version with ids, and
/// `referenced` the latest entry of each other namespace it names. A
/// namespace never written, or retired, declares nothing, so a reference
/// to it is refused; two namespaces that reference each other can't both
/// be written first. This is the one place a namespace write judges
/// another namespace.
fn references(
    own: &NamespaceName,
    named: &Namespace<RelationName, NamespaceName>,
    ids: &Namespace,
    referenced: &[(NamespaceId, Option<Entry>)],
) -> Result<(), NamespaceWriteError> {
    // A rename keeps the order names are first met in, so the two lists
    // pair each name with its id.
    let relations: HashMap<&RelationName, RelationId> = named
        .names()
        .into_iter()
        .zip(ids.names().into_iter().copied())
        .collect();
    let namespaces: HashMap<&NamespaceName, NamespaceId> = named
        .namespaces()
        .into_iter()
        .zip(ids.namespaces().into_iter().copied())
        .collect();
    let declares = |namespace: &NamespaceName, relation: &RelationName| {
        let (Some(namespace), Some(relation)) =
            (namespaces.get(namespace), relations.get(relation))
        else {
            return Some(false);
        };
        let latest = referenced.iter().find(|(id, _)| id == namespace);
        let epochs = latest.and_then(|(_, entry)| entry.as_ref()?.epochs());
        Some(epochs.is_some_and(|epochs| epochs.declares(*relation)))
    };
    let problems = named.references(own, declares);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(NamespaceWriteError::References(problems))
    }
}
