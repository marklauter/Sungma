//! The name ports, and the store failure every context reports. Every call
//! is async and fallible because that is the shape of the real stores.
//!
//! `fn ... -> impl Future<Output = ...> + Send` is an `async fn` in a trait
//! that also promises its future can move between threads. Implementations
//! still write a plain `async fn`.

use std::{future::Future, sync::Arc};

use thiserror::Error;

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

impl<R: NameReader + Send + Sync + ?Sized> NameReader for Arc<R> {
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
