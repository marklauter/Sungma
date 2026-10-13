//! A namespace version: one namespace as declared from a revision on, with
//! the epoch of each relation it declares. See `docs/specs/one-timeline.md`.
//!
//! A version's span opens at its revision. The next version, or the
//! namespace's retirement, closes it, so the store keeps the close.

use std::{collections::HashMap, sync::Arc};

use thiserror::Error;

use crate::{id::RelationId, namespace::Namespace, revision::Revision};

/// A version must come after the one it follows.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("a version at {next:?} can't follow the version at {current:?}")]
pub struct NotLater {
    pub current: Revision,
    pub next: Revision,
}

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
    pub fn first(namespace: Namespace, revision: Revision) -> Self {
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
    pub fn next(&self, namespace: Namespace, revision: Revision) -> Result<Self, NotLater> {
        if revision <= self.revision {
            return Err(NotLater {
                current: self.revision,
                next: revision,
            });
        }
        let epochs = namespace
            .relations()
            .map(|(relation, _)| (*relation, self.epoch(*relation).unwrap_or(revision)))
            .collect();
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

    /// `None` when this version doesn't declare the relation.
    pub fn epoch(&self, relation: RelationId) -> Option<Revision> {
        self.epochs.get(&relation).copied()
    }
}
