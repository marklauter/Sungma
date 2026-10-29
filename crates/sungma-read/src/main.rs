//! `sungma-read` serves the Read service on `SUNGMA_ADDR`,
//! `127.0.0.1:8081` by default.
//!
//! With `SUNGMA_DYNAMODB_URL` set, it reads the table
//! `SUNGMA_DYNAMODB_TABLE`, `sungma` by default, in DynamoDB Local at that
//! URL. The Write service creates that table. Otherwise it runs on an
//! empty memory table.

use std::{env, error::Error, sync::Arc};

use sungma_dynamodb::{Table, items::DynamoTable, local};
use sungma_names::{read::NameRows, store::ReadingDictionary};
use sungma_read::http::{AppState, router};
use sungma_table::{
    memory::MemoryTable,
    read::{Consistency, ItemReader},
};

type Failed = Box<dyn Error>;

#[tokio::main]
async fn main() -> Result<(), Failed> {
    if env::args().len() > 1 {
        return Err("usage: sungma-read".into());
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

async fn serve<E: ItemReader + Clone + Send + Sync + 'static>(table: E) -> Result<(), Failed> {
    let names = ReadingDictionary(NameRows::new(table.clone(), Consistency::Eventual));
    let state = AppState::new(names, table);
    let addr = env::var("SUNGMA_ADDR").unwrap_or_else(|_| "127.0.0.1:8081".to_owned());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("sungma-read listening on {addr}");
    axum::serve(listener, router(Arc::new(state))).await?;
    Ok(())
}
