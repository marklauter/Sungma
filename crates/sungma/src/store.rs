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
    id::{NamespaceId, RelationId},
    revision::Revision,
    rewrite::Rewrite,
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

/// Facts keyed the way a wide-column store keys them: the subjectset is the
/// partition key and the subject is the sort key. Every read is as of a
/// revision.
pub trait FactStore {
    /// The revision of the latest write.
    fn head(&self) -> impl Future<Output = Result<Revision, StoreError>> + Send;

    /// Point lookup: whether `set@subject` is stored.
    fn contains(
        &self,
        set: &Subjectset,
        subject: &Subject,
        revision: Revision,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// The subjectsets stored under `set`: a sort-key range read that skips
    /// identities and resource members.
    fn subjectsets(
        &self,
        set: &Subjectset,
        revision: Revision,
    ) -> impl Future<Output = Result<Vec<Subjectset>, StoreError>> + Send;

    /// Every subject stored under `set`.
    fn subjects(
        &self,
        set: &Subjectset,
        revision: Revision,
    ) -> impl Future<Output = Result<Vec<Subject>, StoreError>> + Send;
}

/// The rewrite of each declared relation, by namespace. Not versioned yet:
/// every read sees the current namespaces. A rewrite is shared, so a cache
/// can evict it while a check still walks it.
pub trait NamespaceStore {
    /// `None` when the namespace or the relation isn't declared.
    fn rewrite(
        &self,
        namespace: NamespaceId,
        relation: RelationId,
    ) -> impl Future<Output = Result<Option<Arc<Rewrite>>, StoreError>> + Send;
}
