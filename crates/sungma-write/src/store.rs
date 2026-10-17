//! The write ports. A [`FactWriter`] keeps the history of each fact and a
//! [`NamespaceWriter`] keeps the versions of each namespace; both stamp
//! their writes from one revision clock, the `Clock` port in
//! `sungma-clock`, so facts and namespace versions share one timeline.
//! [`crate::rows::RowWriter`] serves both over the table.
//!
//! `fn ... -> impl Future<Output = ...> + Send` is an `async fn` in a trait
//! that also promises its future can move between threads. Implementations
//! still write a plain `async fn`.

use std::future::Future;

use thiserror::Error;

use sungma::{
    graph::Fact,
    id::{NamespaceId, RelationId},
    namespace::Namespace,
    revision::Revision,
    store::StoreError,
};
use sungma_clock::ClockFault;

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
    /// The write takes more rows than one write holds, so the caller
    /// splits it.
    #[error("a write of {rows} rows is more than one write holds, {most}")]
    TooLarge { rows: usize, most: usize },
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
    /// A retirement of a namespace with no latest version.
    #[error("the namespace has no latest version to retire")]
    NothingToRetire,
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
