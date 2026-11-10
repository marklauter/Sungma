//! The DynamoDB table passes every table test, on DynamoDB Local.

mod common;

use sungma_dynamodb::items::DynamoTable;
use sungma_table::{
    item::{Item, Value},
    key::Key,
    read::{Consistency, ItemReader, Query},
    table_tests,
    write::{ItemWriter, Step, Work},
};

table_tests!(
    DynamoTable::new(common::table().await),
    DynamoTable::new(common::table().await)
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

/// DynamoDB reads at most a megabyte a page, so a query of larger items
/// takes several pages, and a limit holds across them.
#[tokio::test]
async fn a_query_past_one_page_reads_on_and_keeps_its_limit() {
    let table = DynamoTable::new(common::table().await);
    let large = Item::new().with("text", Value::Text("x".repeat(300_000)));
    let steps = (0..5u8)
        .map(|sort| Step::Insert {
            key: Key {
                partition: b"p".to_vec(),
                sort: vec![sort],
            },
            item: large.clone(),
        })
        .collect();
    table.commit(Work(steps)).await.unwrap();
    let sorts = |items: Vec<(Vec<u8>, Item)>| -> Vec<u8> {
        items.into_iter().map(|(sort, _)| sort[0]).collect()
    };
    let mut query = Query::partition(b"p".to_vec());
    let all = table.query(&query, Consistency::Strong).await.unwrap();
    assert_eq!(sorts(all), [0, 1, 2, 3, 4]);
    query.limit = Some(4);
    let some = table.query(&query, Consistency::Strong).await.unwrap();
    assert_eq!(sorts(some), [0, 1, 2, 3]);
}

/// A test's table is named with a UUID and deleted once the last clone of
/// it is dropped.
#[tokio::test]
async fn a_tests_table_is_deleted_once_dropped() {
    let table = common::table().await;
    let name = table.name().to_owned();
    let uuid = name.strip_prefix("sungma-dynamodb-").unwrap();
    assert_eq!(uuid.len(), 36, "{name}");
    let probe = sungma_dynamodb::Table::new(table.client().clone(), name);
    let clone = table.clone();
    drop(table);
    assert!(probe.exists().await.unwrap());
    drop(clone);
    assert!(!probe.exists().await.unwrap());
}

/// A test that panics still deletes its table.
#[tokio::test]
async fn a_tests_table_is_deleted_when_the_test_panics() {
    let table = common::table().await;
    let probe = sungma_dynamodb::Table::new(table.client().clone(), table.name());
    let failed = tokio::spawn(async move {
        let _held = table;
        panic!("the test failed");
    })
    .await;
    assert!(failed.is_err());
    assert!(!probe.exists().await.unwrap());
}

/// A guard for a table that was never made, or is gone, does nothing.
#[tokio::test]
async fn a_guard_for_no_table_does_nothing() {
    let endpoint =
        std::env::var("SUNGMA_DYNAMODB_URL").unwrap_or_else(|_| "http://localhost:8000".to_owned());
    let name = sungma_dynamodb::testkit::name("sungma-dynamodb");
    drop(sungma_dynamodb::testkit::Scratch::new(&endpoint, &name));
    drop(sungma_dynamodb::testkit::Scratch::new(
        "http://127.0.0.1:9",
        &name,
    ));
    let table = sungma_dynamodb::Table::new(sungma_dynamodb::local(&endpoint), name);
    assert!(!table.exists().await.unwrap());
    assert!(table.delete().await.is_err());
}
