//! The row writer over the DynamoDB table, on DynamoDB Local. Each test
//! gets a table of its own.

mod common;

use sungma_dynamodb::items::DynamoTable;
use sungma_write::testkit::rows::Make;

struct MakeDynamo;

impl Make<DynamoTable> for MakeDynamo {
    async fn make(&self) -> DynamoTable {
        DynamoTable::new(common::table().await)
    }
}

sungma_write::row_writer_tests!(MakeDynamo);
