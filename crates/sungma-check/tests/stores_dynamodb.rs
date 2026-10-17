//! The write and Check contexts over the DynamoDB table run the tests every
//! store runs, on DynamoDB Local. Each test gets a table of its own.

mod common;

use common::{Contexts, table};
use sungma_dynamodb::items::DynamoTable;

async fn dynamo() -> Contexts<DynamoTable, DynamoTable> {
    let table = DynamoTable::new(table().await);
    Contexts::new(table.clone(), table).await
}

// The memory table shrinks the same property fully.
sungma_write::store_tests!(dynamo, cases = 32, shrink = 0);
