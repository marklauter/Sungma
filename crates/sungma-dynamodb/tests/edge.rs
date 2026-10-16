//! The DynamoDB edge passes every edge test, on DynamoDB Local.

mod common;

use sungma_dynamodb::edge::DynamoEdge;
use sungma_edge::edge_tests;

edge_tests!(
    DynamoEdge::new(common::table().await),
    DynamoEdge::new(common::table().await)
);

#[tokio::test]
async fn a_table_knows_whether_it_exists() {
    let table = common::table().await;
    assert!(table.exists().await.unwrap());
    let missing =
        sungma_dynamodb::Table::new(table.client().clone(), format!("{}-missing", table.name()));
    assert!(!missing.exists().await.unwrap());
}

#[tokio::test]
async fn a_table_that_cannot_be_reached_is_a_store_failure() {
    let table = sungma_dynamodb::Table::new(
        sungma_dynamodb::local("http://127.0.0.1:9"),
        "sungma-unreachable",
    );
    assert!(table.exists().await.is_err());
    assert!(table.create().await.is_err());
}
