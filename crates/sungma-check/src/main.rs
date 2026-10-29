//! `sungma-check` serves the Check service on `SUNGMA_ADDR`,
//! `127.0.0.1:8080` by default.
//!
//! With `SUNGMA_DYNAMODB_URL` set, it checks against the table
//! `SUNGMA_DYNAMODB_TABLE`, `sungma` by default, in DynamoDB Local at that
//! URL. The Write service creates that table. Otherwise it runs on an
//! empty memory table.

use std::{env, error::Error, sync::Arc};

use sungma::{
    name::{NamespaceName, RelationName},
    namespace::Namespace,
};
use sungma_check::{
    http::{AppState, router},
    rows::RowReader,
};
use sungma_dynamodb::{Table, items::DynamoTable, local};
use sungma_lang::namespace;
use sungma_names::{read::NameRows, store::ReadingDictionary};
use sungma_table::{
    memory::MemoryTable,
    read::{Consistency, ItemReader},
};

type Failed = Box<dyn Error>;

/// How many namespace versions Check keeps parsed.
const VERSIONS: usize = 1024;

#[tokio::main]
async fn main() -> Result<(), Failed> {
    if env::args().len() > 1 {
        return Err("usage: sungma-check".into());
    }
    match env::var("SUNGMA_DYNAMODB_URL") {
        Ok(url) => {
            let name = env::var("SUNGMA_DYNAMODB_TABLE").unwrap_or_else(|_| "sungma".to_owned());
            let table = Table::new(local(&url), name);
            if !table.exists().await? {
                let name = table.name();
                return Err(
                    format!("table {name} doesn't exist; the Write service creates it").into(),
                );
            }
            serve(DynamoTable::new(table)).await
        }
        Err(_) => serve(Arc::new(MemoryTable::new())).await,
    }
}

fn parse(document: &str) -> Result<(NamespaceName, Namespace<RelationName>), String> {
    namespace::parse(document)
        .map(|document| (document.namespace, document.relations))
        .map_err(|errors| namespace::Refusal(errors).to_string())
}

async fn serve<E: ItemReader + Clone + Send + Sync + 'static>(table: E) -> Result<(), Failed> {
    let names = || NameRows::new(table.clone(), Consistency::Eventual);
    let state = AppState::new(
        ReadingDictionary(names()),
        RowReader::new(table.clone(), names(), parse, VERSIONS),
    );
    let addr = env::var("SUNGMA_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_owned());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("sungma-check listening on {addr}");
    axum::serve(listener, router(Arc::new(state))).await?;
    Ok(())
}
