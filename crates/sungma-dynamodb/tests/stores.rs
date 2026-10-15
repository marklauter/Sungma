//! The DynamoDB stores run the tests every store runs, against DynamoDB
//! Local at `SUNGMA_DYNAMODB_URL`, `http://localhost:8000` by default.
//! `scripts/dynamodb.sh` starts it. Each test gets a table of its own.

mod common;

use sungma_clock::DevClock;
use sungma_dynamodb::{DynamoReader, DynamoWriter};
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

async fn dynamo() -> Dynamo {
    let table = common::table().await;
    Dynamo {
        reader: DynamoReader::new(table.clone()),
        writer: DynamoWriter::dev(table).await.unwrap(),
    }
}

// The in-memory stores shrink the same property fully.
sungma_write::store_tests!(dynamo, cases = 32, shrink = 0);
