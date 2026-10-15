//! Ports to external storage. Every call is async and fallible because
//! that is the shape of the real stores; the in-memory versions in the
//! `sungma-memory` crate simulate them.
//!
//! These are the read ports, shared by every service that reads. The write
//! ports are in the `sungma-write` crate.
//!
//! `fn ... -> impl Future<Output = ...> + Send` is an `async fn` in a trait
//! that also promises its future can move between threads. Implementations
//! still write a plain `async fn`.

use std::{future::Future, sync::Arc};

use thiserror::Error;

use crate::{
    graph::{Subject, Subjectset},
    id::NamespaceId,
    revision::Revision,
    version::Version,
};

#[derive(Debug, Error)]
#[error("store failure: {0}")]
pub struct StoreError(pub String);

/// The pool a name is interned in. Each pool mints its own ids, so one id
/// names different strings in different pools. Only namespace and relation
/// names are interned; see `docs/interner.md`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Pool {
    Namespaces,
    /// One pool for every namespace's relations: `(parent, viewer)` evaluates
    /// `viewer` under whichever namespace a parent fact names, so a relation id
    /// means the same name in every namespace.
    Relations,
}

/// The names stored in each [`Pool`], read by key. The storage under a
/// [`Dictionary`].
pub trait NameReader {
    /// The id stored for `name` in `pool`, `None` when it isn't stored.
    fn id(
        &self,
        pool: Pool,
        name: &str,
    ) -> impl Future<Output = Result<Option<u32>, StoreError>> + Send;

    /// The name stored for `id` in `pool`, `None` when no name has it.
    fn name(
        &self,
        pool: Pool,
        id: u32,
    ) -> impl Future<Output = Result<Option<String>, StoreError>> + Send;
}

/// Maps names to their interned ids and back, within a [`Pool`]. Read-only:
/// Check never mints ids.
pub trait Dictionary {
    /// `None` when the name was never interned in `pool`.
    fn lookup(
        &self,
        pool: Pool,
        name: &str,
    ) -> impl Future<Output = Result<Option<u32>, StoreError>> + Send;

    /// The name an id was minted for, `None` when no name in `pool` has it.
    fn name(
        &self,
        pool: Pool,
        id: u32,
    ) -> impl Future<Output = Result<Option<String>, StoreError>> + Send;
}

/// A [`Dictionary`] that reads each name from a [`NameReader`], for a
/// service that never mints.
#[derive(Debug)]
pub struct ReadingDictionary<R>(pub R);

impl<R: NameReader + Sync> Dictionary for ReadingDictionary<R> {
    async fn lookup(&self, pool: Pool, name: &str) -> Result<Option<u32>, StoreError> {
        self.0.id(pool, name).await
    }

    async fn name(&self, pool: Pool, id: u32) -> Result<Option<String>, StoreError> {
        self.0.name(pool, id).await
    }
}

/// Facts keyed the way a wide-column store keys them: the subjectset is the
/// partition key and the subject is the sort key. Every read is as of a
/// revision, and gives each fact with the revision that wrote it, where
/// the span covering the read opened. Which facts count is the reader's
/// rule, not the store's: see [`Version::counts`].
pub trait FactReader {
    /// The revision of the latest write, of a fact or of a namespace
    /// version: both share one timeline, so a read at the head sees both.
    fn head(&self) -> impl Future<Output = Result<Revision, StoreError>> + Send;

    /// Point lookup: the revision that wrote `set@subject`, `None` when it
    /// isn't stored at `revision`.
    fn written(
        &self,
        set: &Subjectset,
        subject: &Subject,
        revision: Revision,
    ) -> impl Future<Output = Result<Option<Revision>, StoreError>> + Send;

    /// The subjectsets stored under `set`: a sort-key range read that skips
    /// identities and resource members.
    fn subjectsets(
        &self,
        set: &Subjectset,
        revision: Revision,
    ) -> impl Future<Output = Result<Vec<(Subjectset, Revision)>, StoreError>> + Send;

    /// Every subject stored under `set`.
    fn subjects(
        &self,
        set: &Subjectset,
        revision: Revision,
    ) -> impl Future<Output = Result<Vec<(Subject, Revision)>, StoreError>> + Send;
}

/// Namespace versions, keyed by namespace and then by the revision each
/// opens at. A version is shared, so a cache can evict it while a check
/// still walks it.
pub trait NamespaceReader {
    /// The version whose span covers `revision`, `None` when the namespace
    /// wasn't created yet or was retired then.
    fn version(
        &self,
        namespace: NamespaceId,
        revision: Revision,
    ) -> impl Future<Output = Result<Option<Arc<Version>>, StoreError>> + Send;
}
