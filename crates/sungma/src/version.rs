//! A namespace's history: its versions, each with the epoch of every
//! relation it declares, and its retirements. See
//! `docs/specs/one-timeline.md`.
//!
//! Each entry opens at its revision, and the span before it closes there,
//! so every revision is stored once. A [`History`] is the only way to build
//! a [`Version`], and it refuses an entry that doesn't come after the last,
//! so no history is out of order, whatever clock its revisions come from.

use std::{collections::HashMap, sync::Arc};

use thiserror::Error;

use crate::{id::RelationId, namespace::Namespace, revision::Revision};

/// A stored namespace version.
#[derive(Debug)]
pub struct Version {
    namespace: Arc<Namespace>,
    revision: Revision,
    /// Every relation the namespace declares has one.
    epochs: HashMap<RelationId, Revision>,
}

impl Version {
    /// The first version of a namespace, or the first since it was retired.
    /// Every relation it declares gets `revision` as its epoch.
    fn first(namespace: Namespace, revision: Revision) -> Self {
        let epochs = namespace
            .relations()
            .map(|(relation, _)| (*relation, revision))
            .collect();
        Self {
            namespace: Arc::new(namespace),
            revision,
            epochs,
        }
    }

    /// The version after this one. A relation this version declares keeps
    /// its epoch, and any other gets `revision`.
    fn next(&self, namespace: Namespace, revision: Revision) -> Self {
        let epochs = namespace
            .relations()
            .map(|(relation, _)| (*relation, self.epoch(*relation).unwrap_or(revision)))
            .collect();
        Self {
            namespace: Arc::new(namespace),
            revision,
            epochs,
        }
    }

    pub fn namespace(&self) -> &Arc<Namespace> {
        &self.namespace
    }

    /// Where this version's span opens.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// `None` when this version doesn't declare the relation.
    pub fn epoch(&self, relation: RelationId) -> Option<Revision> {
        self.epochs.get(&relation).copied()
    }
}

/// One step in a namespace's history.
#[derive(Clone, Debug)]
pub enum Entry {
    Version(Arc<Version>),
    /// A close with nothing opening after it.
    Retired(Revision),
}

impl Entry {
    /// Where this entry opens, and the span before it closes.
    pub fn revision(&self) -> Revision {
        match self {
            Entry::Version(version) => version.revision(),
            Entry::Retired(revision) => *revision,
        }
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum HistoryError {
    /// Each entry must come after the last.
    #[error("revision {revision:?} doesn't come after {last:?}")]
    NotLater { last: Revision, revision: Revision },
    /// Only a namespace with a current version can be retired.
    #[error("the namespace has no current version to retire")]
    NothingToRetire,
}

/// One namespace's history, oldest first.
#[derive(Debug, Default)]
pub struct History {
    entries: Vec<Entry>,
}

impl History {
    /// The version whose span is still open, `None` when the namespace was
    /// never created or is retired.
    pub fn current(&self) -> Option<&Arc<Version>> {
        match self.entries.last()? {
            Entry::Version(version) => Some(version),
            Entry::Retired(_) => None,
        }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Stores `namespace` as the version after the current one, or as a
    /// first version when there's none, opening at `revision`.
    pub fn write(&mut self, namespace: Namespace, revision: Revision) -> Result<(), HistoryError> {
        self.check(revision)?;
        let version = match self.current() {
            Some(current) => current.next(namespace, revision),
            None => Version::first(namespace, revision),
        };
        self.entries.push(Entry::Version(Arc::new(version)));
        Ok(())
    }

    /// Closes the current version's span at `revision` and opens none.
    pub fn retire(&mut self, revision: Revision) -> Result<(), HistoryError> {
        if self.current().is_none() {
            return Err(HistoryError::NothingToRetire);
        }
        self.check(revision)?;
        self.entries.push(Entry::Retired(revision));
        Ok(())
    }

    fn check(&self, revision: Revision) -> Result<(), HistoryError> {
        match self.entries.last().map(Entry::revision) {
            Some(last) if last >= revision => Err(HistoryError::NotLater { last, revision }),
            _ => Ok(()),
        }
    }
}
