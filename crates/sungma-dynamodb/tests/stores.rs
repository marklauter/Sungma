//! The DynamoDB stores run the tests every store runs, against DynamoDB
//! Local at `SUNGMA_DYNAMODB_URL`, `http://localhost:8000` by default.
//! `scripts/dynamodb.sh` starts it. Each test gets a table of its own.

use std::sync::atomic::{AtomicU64, Ordering};

use sungma_clock::DevClock;
use sungma_dynamodb::{DynamoReader, DynamoWriter, Table, local};
use sungma_write::testkit::Stores;

struct Dynamo {
    reader: DynamoReader,
    writer: DynamoWriter<DevClock>,
}

impl Stores for Dynamo {
    type Reader = DynamoReader;
    type Writer = DynamoWriter<DevClock>;
    type Names = DynamoWriter<DevClock>;

    fn reader(&self) -> &DynamoReader {
        &self.reader
    }

    fn writer(&self) -> &DynamoWriter<DevClock> {
        &self.writer
    }

    fn names(&self) -> &DynamoWriter<DevClock> {
        &self.writer
    }
}

/// A new table, named for this process and a counter so parallel tests
/// and runs don't share one.
async fn table() -> Table {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let endpoint =
        std::env::var("SUNGMA_DYNAMODB_URL").unwrap_or_else(|_| "http://localhost:8000".to_owned());
    let name = format!(
        "sungma-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let table = Table::new(local(&endpoint), name);
    table.create().await.unwrap_or_else(|error| {
        panic!("DynamoDB Local at {endpoint} refused a table; run scripts/dynamodb.sh: {error}")
    });
    table
}

async fn dynamo() -> Dynamo {
    let table = table().await;
    Dynamo {
        reader: DynamoReader::new(table.clone()),
        writer: DynamoWriter::new(table, DevClock::default()),
    }
}

sungma_write::store_tests!(dynamo, cases = 32);
