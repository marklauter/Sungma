//! The write and Check contexts over the DynamoDB edge run the tests every
//! store runs, on DynamoDB Local. Each test gets a table of its own.

mod common;

use common::{Contexts, table};
use sungma_dynamodb::edge::DynamoEdge;

async fn dynamo() -> Contexts<DynamoEdge, DynamoEdge> {
    let edge = DynamoEdge::new(table().await);
    Contexts::new(edge.clone(), edge).await
}

// The memory edge shrinks the same property fully.
sungma_write::store_tests!(dynamo, cases = 32, shrink = 0);
