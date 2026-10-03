//! Ports to external storage. Every call is async and fallible because
//! that is the shape of the real stores; the in-memory versions in
//! [`crate::memory`] simulate them.
//!
//! `fn ... -> impl Future<Output = ...> + Send` is an `async fn` in a trait
//! that also promises its future can move between threads. Implementations
//! still write a plain `async fn`.

use std::future::Future;

use thiserror::Error;

use crate::model::{Subject, Subjectset};

#[derive(Debug, Error)]
#[error("store failure: {0}")]
pub struct StoreError(pub String);

/// Maps names to their interned ids. Read-only: Check never mints ids.
pub trait Dictionary {
    /// `None` when the name was never interned.
    fn lookup(&self, name: &str) -> impl Future<Output = Result<Option<u32>, StoreError>> + Send;
}

pub trait FactStore {
    /// The subjects of the facts stored under `set`.
    fn subjects(
        &self,
        set: Subjectset,
    ) -> impl Future<Output = Result<Vec<Subject>, StoreError>> + Send;
}
