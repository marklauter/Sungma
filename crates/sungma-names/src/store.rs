//! The name ports: reading names, minting ids for them, and the storage
//! ids are minted through. Every call is async and fallible because that
//! is the shape of the real stores.
//!
//! `fn ... -> impl Future<Output = ...> + Send` is an `async fn` in a trait
//! that also promises its future can move between threads. Implementations
//! still write a plain `async fn`.

use std::{future::Future, ops::Range};

use sungma::store::StoreError;
use thiserror::Error;

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

impl<R: NameReader + Sync + ?Sized> NameReader for &R {
    fn id(
        &self,
        pool: Pool,
        name: &str,
    ) -> impl Future<Output = Result<Option<u32>, StoreError>> + Send {
        R::id(self, pool, name)
    }

    fn name(
        &self,
        pool: Pool,
        id: u32,
    ) -> impl Future<Output = Result<Option<String>, StoreError>> + Send {
        R::name(self, pool, id)
    }
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
