//! `sungma-api [<facts.json> <namespace.json>...]` serves the API on
//! `SUNGMA_ADDR`, `127.0.0.1:8080` by default.
//!
//! With `SUNGMA_DYNAMODB_URL` set, it runs on DynamoDB Local at that URL,
//! on the table `SUNGMA_DYNAMODB_TABLE`, `sungma` by default, created if
//! it's missing. Otherwise it runs in memory.
//!
//! The files are optional seed data: a fact document and at least one
//! namespace document, written through the API's own write services into
//! a new store. A table that already exists isn't seeded again.

use std::{env, error::Error, fs, sync::Arc};

use sungma_api::{AppState, Ids, Items, Stamps, router};
use sungma_dynamodb::{Table, items::DynamoTable, local};
use sungma_node::Node;

type Failed = Box<dyn Error>;

#[tokio::main]
async fn main() -> Result<(), Failed> {
    let args: Vec<String> = env::args().skip(1).collect();
    // At least one namespace: with none, the facts couldn't be written.
    let seed = match args.as_slice() {
        [] => None,
        [facts, namespaces @ ..] if !namespaces.is_empty() => Some((facts, namespaces)),
        _ => return Err("usage: sungma-api [<facts.json> <namespace.json>...]".into()),
    };
    let seed = match seed {
        Some((facts, namespaces)) => Some((
            fs::read_to_string(facts)?,
            namespaces
                .iter()
                .map(fs::read_to_string)
                .collect::<Result<Vec<_>, _>>()?,
        )),
        None => None,
    };
    match env::var("SUNGMA_DYNAMODB_URL") {
        Ok(url) => {
            let name = env::var("SUNGMA_DYNAMODB_TABLE").unwrap_or_else(|_| "sungma".to_owned());
            let table = Table::new(local(&url), name);
            // A table that exists was seeded when it was made, so it's
            // served as it is.
            let seed = if table.exists().await? {
                if seed.is_some() {
                    println!("table {} exists, so it isn't seeded", table.name());
                }
                None
            } else {
                table.create().await?;
                seed
            };
            let table = DynamoTable::new(table);
            serve(AppState::new(Node::dev(table.clone(), table).await?), seed).await
        }
        Err(_) => serve(AppState::memory(), seed).await,
    }
}

async fn serve<E: Items, S: Ids, C: Stamps>(
    state: AppState<E, S, C>,
    seed: Option<(String, Vec<String>)>,
) -> Result<(), Failed> {
    if let Some((facts, namespaces)) = &seed {
        let namespaces: Vec<&str> = namespaces.iter().map(String::as_str).collect();
        state.seed(&namespaces, facts).await?;
    }
    let addr = env::var("SUNGMA_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_owned());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("sungma-api listening on {addr}");
    axum::serve(listener, router(Arc::new(state))).await?;
    Ok(())
}
