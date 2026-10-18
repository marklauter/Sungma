//! The store failure every context reports.

use thiserror::Error;

#[derive(Debug, Error)]
#[error("store failure: {0}")]
pub struct StoreError(pub String);
