//! Ports to external storage. Every call is async and fallible because
//! that is the shape of the real stores; the in-memory versions in
//! [`crate::memory`] simulate them.
//!
//! `fn ... -> impl Future<Output = ...> + Send` is an `async fn` in a trait
//! that also promises its future can move between threads. Implementations
//! still write a plain `async fn`.

use std::{future::Future, sync::Arc};

use thiserror::Error;

use crate::{
    model::{RelationId, Revision, Subject, Subjectset, TheoryId},
    rewrite::Rewrite,
};

#[derive(Debug, Error)]
#[error("store failure: {0}")]
pub struct StoreError(pub String);

/// Maps names to their interned ids. Read-only: Check never mints ids.
pub trait Dictionary {
    /// `None` when the name was never interned.
    fn lookup(&self, name: &str) -> impl Future<Output = Result<Option<u32>, StoreError>> + Send;
}

/// Facts keyed the way a wide-column store keys them: the subjectset is the
/// partition key and the subject is the sort key. Every read is as of a
/// revision.
pub trait FactStore {
    /// The revision of the latest write.
    fn head(&self) -> impl Future<Output = Result<Revision, StoreError>> + Send;

    /// Point lookup: whether `set@subject` is stored.
    fn contains(
        &self,
        set: Subjectset,
        subject: Subject,
        revision: Revision,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// The subjectsets stored under `set`: a sort-key range read that skips
    /// identities and resource members.
    fn subjectsets(
        &self,
        set: Subjectset,
        revision: Revision,
    ) -> impl Future<Output = Result<Vec<Subjectset>, StoreError>> + Send;

    /// Every subject stored under `set`.
    fn subjects(
        &self,
        set: Subjectset,
        revision: Revision,
    ) -> impl Future<Output = Result<Vec<Subject>, StoreError>> + Send;
}

/// The rewrite of each declared relation, by theory. Not versioned yet:
/// every read sees the current theories. A rewrite is shared, so a cache
/// can evict it while a check still walks it.
pub trait TheoryStore {
    /// `None` when the theory or the relation isn't declared.
    fn rewrite(
        &self,
        theory: TheoryId,
        relation: RelationId,
    ) -> impl Future<Output = Result<Option<Arc<Rewrite>>, StoreError>> + Send;
}
