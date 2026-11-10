//! Tables for tests on DynamoDB Local. Each is named with a random UUID,
//! so no run meets a table an earlier run left, and is deleted when the
//! test is done with it, whether it passed or failed.

use std::thread;

use sungma_table::TableError;
use uuid::Uuid;

use crate::{Table, local};

/// `prefix`, a hyphen and a random UUID: a table name no other test uses.
pub fn name(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::new_v4())
}

/// A new table on DynamoDB Local at `endpoint`, named by [`name`], and
/// deleted once it and every clone of it are dropped.
pub async fn table(endpoint: &str, prefix: &str) -> Result<Table, TableError> {
    let name = name(prefix);
    let scratch = Scratch::new(endpoint, &name);
    let table = Table::new(local(endpoint), name);
    table.create().await?;
    Ok(table.scratch(scratch))
}

/// Deletes the table it names when it's dropped, such as when a test that
/// holds it ends or panics. A table that's already gone, or a store that
/// can't be reached, is left as it is.
#[derive(Debug)]
pub struct Scratch {
    endpoint: String,
    name: String,
}

impl Scratch {
    pub fn new(endpoint: &str, name: &str) -> Self {
        Self {
            endpoint: endpoint.to_owned(),
            name: name.to_owned(),
        }
    }
}

impl Drop for Scratch {
    /// Deletes on a thread of its own, with a runtime of its own, since
    /// drop can't wait on the runtime the test runs on.
    fn drop(&mut self) {
        let (endpoint, name) = (self.endpoint.clone(), self.name.clone());
        let deleted = thread::spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            let _ = runtime.block_on(Table::new(local(&endpoint), name).delete());
        });
        let _ = deleted.join();
    }
}
