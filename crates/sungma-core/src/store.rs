//! Ports to external storage. Every call is async and fallible because
//! that is the shape of the real stores; the in-memory versions in
//! [`crate::memory`] simulate them.
//!
//! `fn ... -> impl Future<Output = ...> + Send` is an `async fn` in a trait
//! that also promises its future can move between threads. Implementations
//! still write a plain `async fn`.

use std::future::Future;

use thiserror::Error;

use crate::model::{Revision, Subject, Subjectset};

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
