//! Namespace versions, each with the epoch of every relation it declares.
//! See `docs/specs/one-timeline.md`.
//!
//! A writer gives a new version its [`Epochs`] with [`Epochs::following`],
//! and a reader builds the [`Version`] it reads back with
//! [`Version::stored`].

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
