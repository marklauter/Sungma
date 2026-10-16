//! The store edge: items under keys, with no knowledge of what they mean.
//! A reader gets an item by its key or queries a partition, a writer
//! commits a unit of work, and a sequence hands out numbers. Each context
//! decides its own keys and items and maps them to its own shapes.
//!
//! [`memory::MemoryTable`] keeps items in a sorted map, for tests. The
//! DynamoDB edge is in `sungma-dynamodb`.

pub mod item;
pub mod key;
pub mod memory;
pub mod read;
pub mod sequence;
pub mod write;

#[cfg(feature = "testkit")]
pub mod testkit;

use thiserror::Error;

/// The store failed, as its message says.
#[derive(Clone, PartialEq, Eq, Debug, Error)]
#[error("store failure: {0}")]
pub struct EdgeError(pub String);
