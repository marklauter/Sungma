//! The write ports. Where a guarantee spans nodes, Sungma runs the protocol
//! and a store only supplies primitives every store has: a [`FactWriter`]
//! keeps the history of each fact, and [`crate::intern::LeasingInterner`]
//! mints ids over a [`NameStore`].
//!
//! `fn ... -> impl Future<Output = ...> + Send` is an `async fn` in a trait
//! that also promises its future can move between threads. Implementations
//! still write a plain `async fn`.

use std::{future::Future, ops::Range};

use thiserror::Error;

use sungma::{
    graph::Fact,
    revision::Revision,
    store::{Dictionary, FactStore, Pool, StoreError},
};

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
pub trait NameStore: Dictionary {
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

/// One change to the stored facts.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum FactWrite {
    /// Stores the fact. Storing a fact already stored changes nothing.
    Insert(Fact),
    /// Deletes the fact. Deleting a fact not stored changes nothing.
    Delete(Fact),
}

/// Writes facts. Sungma keeps the history itself rather than relying on the
/// store's own versioning: a fact is stored with the revision that wrote it
/// and, once deleted, the revision that deleted it, and is never removed.
/// A read at revision R sees the facts written at or before R and not
/// deleted at or before R, so a replay can read any past revision.
///
/// Revisions are issued in commit order. Once a write returns revision R, a
/// read at R sees that write and every write before it, and no write after
/// it. An adapter meets this however its store allows: SQLite's single
/// writer, a counter updated in the write's transaction, or commit
/// timestamps.
pub trait FactWriter: FactStore {
    /// Applies `writes` atomically at the next revision and returns it.
    fn write(
        &self,
        writes: &[FactWrite],
    ) -> impl Future<Output = Result<Revision, StoreError>> + Send;
}
