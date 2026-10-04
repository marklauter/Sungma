//! Ports to external storage. Every call is async and fallible because
//! that is the shape of the real stores; the in-memory versions in
//! [`crate::memory`] simulate them.
//!
//! Reads and writes are separate ports: Check only reads, so it takes a
//! [`Dictionary`] and a [`FactStore`], and writes take an [`Interner`] and
//! a [`FactWriter`].
//!
//! Where a guarantee spans nodes, Sungma runs the protocol and a store only
//! supplies primitives every store has: a [`FactWriter`] keeps the history
//! of each fact, and [`crate::intern::LeasingInterner`] mints ids over a
//! [`NameStore`].
//!
//! `fn ... -> impl Future<Output = ...> + Send` is an `async fn` in a trait
//! that also promises its future can move between threads. Implementations
//! still write a plain `async fn`.

use std::{future::Future, ops::Range, sync::Arc};

use thiserror::Error;

use crate::{
    model::{Fact, RelationId, Revision, Subject, Subjectset, TheoryId},
    rewrite::Rewrite,
};

#[derive(Debug, Error)]
#[error("store failure: {0}")]
pub struct StoreError(pub String);

/// Maps names to their interned ids and back. Read-only: Check never mints
/// ids.
pub trait Dictionary {
    /// `None` when the name was never interned.
    fn lookup(&self, name: &str) -> impl Future<Output = Result<Option<u32>, StoreError>> + Send;

    /// The name an id was minted for, `None` when no name has it.
    fn name(&self, id: u32) -> impl Future<Output = Result<Option<String>, StoreError>> + Send;
}

/// Mints ids for names. Every node that interns a name gets the same id,
/// and an id, once minted, always names the same string. Ids are unique,
/// not dense: an id may be skipped and is never reused.
pub trait Interner: Dictionary {
    /// The name's id, minted if the name is new.
    fn intern(&self, name: &str) -> impl Future<Output = Result<u32, StoreError>> + Send;
}

/// The storage an id is minted through. Both writes are atomic in the
/// store, which is all the coordination minting needs.
pub trait NameStore: Dictionary {
    /// Stores `name` as `id` unless the name is stored already, and returns
    /// the name's id afterwards: `id`, or the id another node stored first.
    fn insert_if_absent(
        &self,
        name: &str,
        id: u32,
    ) -> impl Future<Output = Result<u32, StoreError>> + Send;

    /// Advances the shared id counter by up to `count` and returns the ids
    /// it passed over, which no other lease will return. Empty once the ids
    /// run out.
    fn lease(&self, count: u32) -> impl Future<Output = Result<Range<u32>, StoreError>> + Send;
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

/// One change to the stored facts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
