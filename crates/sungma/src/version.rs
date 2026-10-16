//! A namespace's history: its versions, each with the epoch of every
//! relation it declares, and its retirements. See
//! `docs/specs/one-timeline.md`.
//!
//! Each entry opens at its revision, and the span before it closes there,
//! so every revision is stored once. A [`History`] is the only way to build
//! a new [`Version`], and it refuses an entry that doesn't come after the
//! last, so no history is out of order, whatever clock its revisions come
//! from. A store reads a version back with [`Version::stored`], and picks a
//! history up from its latest entry with [`History::resume`].

use std::{collections::HashMap, sync::Arc};

use thiserror::Error;

use crate::{id::RelationId, namespace::Namespace, revision::Revision};

/// The epoch of each relation a namespace version declares: the revision
/// it was last declared at. A fact counts only when its relation has an
/// epoch and its span opened at or after it.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Epochs(HashMap<RelationId, Revision>);

impl Epochs {
    pub fn new(epochs: HashMap<RelationId, Revision>) -> Self {
        Self(epochs)
    }

    /// `None` when the relation isn't declared.
    pub fn epoch(&self, relation: RelationId) -> Option<Revision> {
        self.0.get(&relation).copied()
    }

    pub fn declares(&self, relation: RelationId) -> bool {
        self.0.contains_key(&relation)
    }

    /// Whether a fact under `relation` whose span opened at `written`
    /// counts: the relation is declared, and the span opened at or after
    /// its epoch.
    pub fn counts(&self, relation: RelationId, written: Revision) -> bool {
        self.epoch(relation).is_some_and(|epoch| written >= epoch)
    }

    /// The epochs of a version opening at `at` that declares `declared`,
    /// after `previous`, the epochs of the version before it, `None` when
    /// it's the first or follows a retirement. A relation `previous`
    /// declares keeps its epoch, and any other gets `at`.
    pub fn following(
        previous: Option<&Epochs>,
        declared: impl IntoIterator<Item = RelationId>,
        at: Revision,
    ) -> Self {
        Self(
            declared
                .into_iter()
                .map(|relation| {
                    let epoch = previous.and_then(|previous| previous.epoch(relation));
                    (relation, epoch.unwrap_or(at))
                })
                .collect(),
        )
    }

    /// Each declared relation and its epoch, in no order.
    pub fn iter(&self) -> impl Iterator<Item = (RelationId, Revision)> + '_ {
        self.0.iter().map(|(relation, epoch)| (*relation, *epoch))
    }
}

/// A stored namespace version.
#[derive(Debug)]
pub struct Version {
    namespace: Arc<Namespace>,
    revision: Revision,
    /// Every relation the namespace declares has one.
    epochs: Epochs,
}

impl Version {
    /// The first version of a namespace, or the first since it was retired.
    /// Every relation it declares gets `revision` as its epoch.
    fn first(namespace: Namespace, revision: Revision) -> Self {
        Self::after(None, namespace, revision)
    }

    fn after(previous: Option<&Epochs>, namespace: Namespace, revision: Revision) -> Self {
        let declared = namespace.relations().map(|(relation, _)| *relation);
        Self {
            epochs: Epochs::following(previous, declared, revision),
            namespace: Arc::new(namespace),
            revision,
        }
    }

    /// The version after this one. A relation this version declares keeps
    /// its epoch, and any other gets `revision`.
    fn next(&self, namespace: Namespace, revision: Revision) -> Self {
        Self::after(Some(&self.epochs), namespace, revision)
    }

    /// A version a store wrote, read back with the epochs it stored.
    /// Refused unless every relation the namespace declares has an epoch,
    /// no other relation has one, and none is later than `revision`.
    pub fn stored(
        namespace: Namespace,
        revision: Revision,
        epochs: Epochs,
    ) -> Result<Self, StoredVersionError> {
        for (relation, _) in namespace.relations() {
            let epoch = epochs
                .epoch(*relation)
                .ok_or(StoredVersionError::NoEpoch(*relation))?;
            if epoch > revision {
                return Err(StoredVersionError::LaterEpoch(*relation));
            }
        }
        if let Some((relation, _)) = epochs
            .iter()
            .find(|(relation, _)| namespace.rewrite(relation).is_none())
        {
            return Err(StoredVersionError::Undeclared(relation));
        }
        Ok(Self {
            namespace: Arc::new(namespace),
            revision,
            epochs,
        })
    }

    pub fn namespace(&self) -> &Arc<Namespace> {
        &self.namespace
    }

    /// Where this version's span opens.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn epochs(&self) -> &Epochs {
        &self.epochs
    }

    /// `None` when this version doesn't declare the relation.
    pub fn epoch(&self, relation: RelationId) -> Option<Revision> {
        self.epochs.epoch(relation)
    }

    pub fn declares(&self, relation: RelationId) -> bool {
        self.epochs.declares(relation)
    }

    /// See [`Epochs::counts`].
    pub fn counts(&self, relation: RelationId, written: Revision) -> bool {
        self.epochs.counts(relation, written)
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

/// Why [`Version::stored`] refused what a store read back.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum StoredVersionError {
    #[error("relation {0:?} is declared but has no epoch")]
    NoEpoch(RelationId),
    #[error("relation {0:?} has an epoch but isn't declared")]
    Undeclared(RelationId),
    #[error("relation {0:?} has an epoch later than its version")]
    LaterEpoch(RelationId),
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum HistoryError {
    /// Each entry must come after the last.
    #[error("revision {revision:?} doesn't come after {last:?}")]
    NotLater { last: Revision, revision: Revision },
    /// Only a namespace with a latest version can be retired.
    #[error("the namespace has no latest version to retire")]
    NothingToRetire,
}

/// One namespace's history, oldest first.
#[derive(Debug, Default)]
pub struct History {
    entries: Vec<Entry>,
}

impl History {
    /// A history that goes on from `latest`, a store's latest entry, so the
    /// next entry gets its epochs from it and must come after it.
    pub fn resume(latest: Entry) -> Self {
        Self {
            entries: vec![latest],
        }
    }

    /// The version whose span is still open, `None` when the namespace was
    /// never created or is retired.
    pub fn latest(&self) -> Option<&Arc<Version>> {
        match self.entries.last()? {
            Entry::Version(version) => Some(version),
            Entry::Retired(_) => None,
        }
    }

    /// The version whose span covers `revision`, `None` when the namespace
    /// wasn't created yet or was retired then.
    pub fn at(&self, revision: Revision) -> Option<&Arc<Version>> {
        let opened = self
            .entries
            .partition_point(|entry| entry.revision() <= revision);
        match self.entries.get(opened.checked_sub(1)?)? {
            Entry::Version(version) => Some(version),
            Entry::Retired(_) => None,
        }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Stores `namespace` as the version after the latest one, or as a
    /// first version when there's none, opening at `revision`.
    pub fn write(&mut self, namespace: Namespace, revision: Revision) -> Result<(), HistoryError> {
        self.check(revision)?;
        let version = match self.latest() {
            Some(latest) => latest.next(namespace, revision),
            None => Version::first(namespace, revision),
        };
        self.entries.push(Entry::Version(Arc::new(version)));
        Ok(())
    }

    /// Closes the latest version's span at `revision` and opens none.
    pub fn retire(&mut self, revision: Revision) -> Result<(), HistoryError> {
        if self.latest().is_none() {
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
