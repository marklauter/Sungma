//! The write ports. Where a guarantee spans nodes, Sungma runs the protocol
//! and a store only supplies primitives every store has: a [`FactWriter`]
//! keeps the history of each fact, a [`NamespaceWriter`] keeps the versions
//! of each namespace, and [`crate::intern::LeasingInterner`] mints ids over
//! a [`NameWriter`]. Both writers stamp their writes from one revision
//! clock, the `Clock` port in `sungma-clock`, so facts and namespace
//! versions share one timeline.
//!
//! `fn ... -> impl Future<Output = ...> + Send` is an `async fn` in a trait
//! that also promises its future can move between threads. Implementations
//! still write a plain `async fn`.

use std::{future::Future, ops::Range, sync::Arc};

use thiserror::Error;

use sungma::{
    graph::Fact,
    id::{NamespaceId, RelationId},
    namespace::Namespace,
    revision::Revision,
    store::{Dictionary, Pool, StoreError},
    version::HistoryError,
};
use sungma_clock::ClockFault;

#[derive(Debug, Error)]
pub enum InternError {
    /// Every id in the pool has been leased. Unlike a store failure, trying
    /// again won't help.
    #[error("no ids left to lease in {0:?}")]
    Exhausted(Pool),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Mints ids for names. Every node that interns a name in a pool gets the
/// same id, and an id, once minted, always names the same string. Ids are
/// unique within their pool, not dense: an id may be skipped and is never
/// reused.
pub trait Interner: Dictionary {
    /// The name's id in `pool`, minted if the name is new there.
    fn intern(
        &self,
        pool: Pool,
        name: &str,
    ) -> impl Future<Output = Result<u32, InternError>> + Send;
}

/// The storage an id is minted through. Both writes are atomic in the
/// store, which is all the coordination minting needs.
pub trait NameWriter {
    /// Stores `name` as `id` in `pool` unless the name is stored there
    /// already, and returns the name's id afterwards: `id`, or the id
    /// another node stored first.
    fn insert_if_absent(
        &self,
        pool: Pool,
        name: &str,
        id: u32,
    ) -> impl Future<Output = Result<u32, StoreError>> + Send;

    /// Advances `pool`'s shared id counter by up to `count` and returns the
    /// ids it passed over, which no other lease will return. Empty once the
    /// pool's ids run out.
    fn lease(
        &self,
        pool: Pool,
        count: u32,
    ) -> impl Future<Output = Result<Range<u32>, StoreError>> + Send;
}

impl<W: NameWriter + Sync + ?Sized> NameWriter for &W {
    fn insert_if_absent(
        &self,
        pool: Pool,
        name: &str,
        id: u32,
    ) -> impl Future<Output = Result<u32, StoreError>> + Send {
        W::insert_if_absent(self, pool, name, id)
    }

    fn lease(
        &self,
        pool: Pool,
        count: u32,
    ) -> impl Future<Output = Result<Range<u32>, StoreError>> + Send {
        W::lease(self, pool, count)
    }
}

impl<W: NameWriter + Send + Sync + ?Sized> NameWriter for Arc<W> {
    fn insert_if_absent(
        &self,
        pool: Pool,
        name: &str,
        id: u32,
    ) -> impl Future<Output = Result<u32, StoreError>> + Send {
        W::insert_if_absent(self, pool, name, id)
    }

    fn lease(
        &self,
        pool: Pool,
        count: u32,
    ) -> impl Future<Output = Result<Range<u32>, StoreError>> + Send {
        W::lease(self, pool, count)
    }
}

/// One change to the stored facts.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum FactWrite {
    /// Stores the fact. Storing a fact already stored changes nothing.
    Insert(Fact),
    /// Retires the fact. Retiring a fact not stored changes nothing.
    Retire(Fact),
}

#[derive(Debug, Error)]
pub enum FactWriteError {
    /// The latest namespace version at the write doesn't declare the
    /// fact's relation, or a subjectset subject's namespace doesn't declare
    /// the subject's relation. The fact would never count.
    #[error("the fact names a relation its namespace doesn't declare")]
    Undeclared(Fact),
    /// The clock can't give a revision it trusts.
    #[error(transparent)]
    Clock(#[from] ClockFault),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Writes facts. Sungma keeps the history itself rather than relying on the
/// store's own versioning: a fact is stored with the revision that wrote it
/// and, once retired, the revision that retired it, and is never removed.
/// A read at revision R sees the facts written at or before R and not
/// retired at or before R, so a replay can read any past revision.
///
/// A write is stamped with a revision. Once it returns revision R, a read
/// at R sees that write and every write acknowledged before it began.
///
/// An insert is refused unless the latest namespace versions declare the
/// fact's relation and, for a subjectset subject, the subject's relation.
/// Inserting a fact whose stored span was written before its relation's
/// epoch closes that span and opens a new one, so a grant asserted again
/// after its relation comes back counts. [`crate::fact`] has both rules,
/// for every store to apply. See `docs/specs/one-timeline.md`.
pub trait FactWriter {
    /// Applies `writes` atomically and returns the revision they're stamped
    /// with, or refuses them all and stores nothing.
    fn write(
        &self,
        writes: &[FactWrite],
    ) -> impl Future<Output = Result<Revision, FactWriteError>> + Send;
}

/// One change to a namespace. See `docs/specs/namespace-writes.md`.
///
/// A write that follows a version names it by its revision, the version the
/// caller read, as an etag names what a write expects to replace.
///
/// Generic over how relations are named, as [`Namespace`] is: a store
/// writes interned ids, and a caller names its relations.
#[derive(Debug)]
pub enum NamespaceWrite<R = RelationId> {
    /// Stores the first version of a namespace that has no latest version.
    Create(Namespace<R>),
    /// Stores the next version, closing the latest version's span.
    NewVersion {
        based_on: Revision,
        namespace: Namespace<R>,
    },
    /// Closes the latest version's span and stores no new version.
    Retire { based_on: Revision },
}

#[derive(Debug, Error)]
pub enum NamespaceWriteError {
    /// The namespace isn't at the version the write was based on. `latest`
    /// is its latest version's revision, `None` when it has none. The
    /// caller reads the latest version and tries again.
    #[error("the namespace isn't at the version the write was based on")]
    Conflict { latest: Option<Revision> },
    /// The namespace's history refuses the revision the store stamped.
    #[error(transparent)]
    History(#[from] HistoryError),
    /// The clock can't give a revision it trusts.
    #[error(transparent)]
    Clock(#[from] ClockFault),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Writes namespace versions. A stored version is never edited: a write
/// stores a new version, closes the latest version's span, or both, and a
/// read at any earlier revision still finds the version it found before.
///
/// Each write is a conditional write on the namespace, so of two writes
/// based on the same version, exactly one succeeds. A namespace write never
/// reads or rewrites facts.
pub trait NamespaceWriter {
    /// Applies `write` to `namespace` atomically and returns the revision it's
    /// stamped with, or refuses it and stores nothing.
    fn write_namespace(
        &self,
        namespace: NamespaceId,
        write: NamespaceWrite,
    ) -> impl Future<Output = Result<Revision, NamespaceWriteError>> + Send;
}
