//! The API's reads: facts and namespace documents at a snapshot, read
//! from their rows and named as a caller names them. A read applies the
//! rule every reader applies: a fact counts only when its namespace's
//! version at the read declares its relation and the fact was written at
//! or after the relation's epoch. See `docs/specs/one-timeline.md`.

pub mod facts;
pub mod namespaces;

use sungma::{revision::Revision, store::StoreError};
use sungma_schema::head;
use sungma_table::read::{Consistency, ItemReader};
use thiserror::Error;

/// Every read is eventual: a read is a snapshot, stale once it's taken.
const READ: Consistency = Consistency::Eventual;

#[derive(Debug, Error)]
pub enum ReadError {
    /// The zookie names a revision past the latest.
    #[error("zookie {zookie:?} is ahead of the latest revision {head:?}")]
    Ahead { zookie: Revision, head: Revision },
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// The revision a read is made at: the latest, which must be at least as
/// fresh as the zookie.
pub async fn revision<R: ItemReader>(
    items: &R,
    zookie: Option<Revision>,
) -> Result<Revision, ReadError> {
    let head = head::read(items, READ).await?.unwrap_or(Revision(0));
    match zookie {
        Some(zookie) if zookie > head => Err(ReadError::Ahead { zookie, head }),
        _ => Ok(head),
    }
}
