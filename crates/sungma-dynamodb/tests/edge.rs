//! The DynamoDB edge passes every edge test, on DynamoDB Local.

mod common;

use sungma_dynamodb::edge::DynamoEdge;
use sungma_edge::edge_tests;

edge_tests!(
    DynamoEdge::new(common::table().await),
    DynamoEdge::new(common::table().await)
);
