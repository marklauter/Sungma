//! The write and Check contexts over the DynamoDB table run the tests every
//! store runs, on DynamoDB Local. Each test gets a table of its own.

mod common;

use sungma_dynamodb::items::DynamoTable;
use sungma_write::testkit::Contexts;

async fn dynamo() -> Contexts<DynamoTable, DynamoTable> {
    let table = DynamoTable::new(common::table().await);
    Contexts::new(table.clone(), table).await
}

// The memory table shrinks the same property fully.
sungma_write::store_tests!(dynamo, cases = 32, shrink = 0);
