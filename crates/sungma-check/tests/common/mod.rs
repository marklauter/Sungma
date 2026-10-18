//! The write and Check contexts over one table, as the shared store tests
//! take them, and a table of its own for each DynamoDB test.

// Each test file compiles this module separately and uses only part of it.
#![allow(dead_code, unused_imports)]

use std::sync::atomic::{AtomicU64, Ordering};

use sungma_dynamodb::{Table, local};

pub use sungma_write::testkit::Contexts;

/// A new table on DynamoDB Local at `SUNGMA_DYNAMODB_URL`,
/// `http://localhost:8000` by default.
pub async fn table() -> Table {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let url =
        std::env::var("SUNGMA_DYNAMODB_URL").unwrap_or_else(|_| "http://localhost:8000".to_owned());
    let name = format!(
        "sungma-check-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let table = Table::new(local(&url), name);
    table.create().await.unwrap_or_else(|error| {
        panic!("DynamoDB Local at {url} refused a table; run scripts/dynamodb.sh: {error}")
    });
    table
}
