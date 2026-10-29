//! `sungma-write [<facts.json> <namespace.json>...]` serves the Write
//! service on `SUNGMA_ADDR`, `127.0.0.1:8082` by default.
//!
//! With `SUNGMA_DYNAMODB_URL` set, it runs on DynamoDB Local at that URL,
//! on the table `SUNGMA_DYNAMODB_TABLE`, `sungma` by default, created if
//! it's missing. Otherwise it runs in memory.
//!
//! The files are optional seed data: a fact document and at least one
//! namespace document, written through the write services into a new
//! store. A table that already exists isn't seeded again.

use std::{env, error::Error, fs, num::NonZeroU32, sync::Arc};

use sungma::{
    name::{NamespaceName, RelationName},
    namespace::Namespace,
};
use sungma_dynamodb::{Table, items::DynamoTable, local};
use sungma_lang::{fact, namespace};
use sungma_names::{intern::LeasingInterner, read::NameRows, write::NameRowWriter};
use sungma_table::{
    memory::{MemorySequence, MemoryTable},
    read::{Consistency, ItemReader},
    sequence::Sequence,
    write::ItemWriter,
};
use sungma_write::{
    http::{AppState, router},
    rows::RowWriter,
    service,
};

type Failed = Box<dyn Error>;

/// How many ids a lease takes at a time.
const LEASE: NonZeroU32 = NonZeroU32::new(64).unwrap();

#[tokio::main]
async fn main() -> Result<(), Failed> {
    let args: Vec<String> = env::args().skip(1).collect();
    // At least one namespace: with none, the facts couldn't be written.
    let seed = match args.as_slice() {
        [] => None,
        [facts, namespaces @ ..] if !namespaces.is_empty() => Some((facts, namespaces)),
        _ => return Err("usage: sungma-write [<facts.json> <namespace.json>...]".into()),
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
            serve(table.clone(), table, seed).await
        }
        Err(_) => {
            let table = Arc::new(MemoryTable::new());
            serve(table, Arc::new(MemorySequence::new()), seed).await
        }
    }
}

/// The printer's canonical document.
fn print(name: &NamespaceName, relations: &Namespace<RelationName>) -> Result<String, String> {
    namespace::print(name, relations).map_err(|error| error.to_string())
}

async fn serve<E, S>(
    table: E,
    sequence: S,
    seed: Option<(String, Vec<String>)>,
) -> Result<(), Failed>
where
    E: ItemReader + ItemWriter + Clone + Send + Sync + 'static,
    S: Sequence + Send + Sync + 'static,
{
    let names = || NameRows::new(table.clone(), Consistency::Strong);
    let interner = LeasingInterner::new(
        names(),
        NameRowWriter::new(table.clone(), table.clone(), sequence),
        LEASE,
    );
    let writer = RowWriter::dev(table.clone(), table.clone(), names(), print).await?;
    if let Some((facts, documents)) = &seed {
        let namespaces = documents
            .iter()
            .map(|document| {
                namespace::parse(document)
                    .map(|document| (document.namespace, document.relations))
                    .map_err(namespace::Refusal)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let facts = fact::parse(facts)?;
        service::seed(&interner, &writer, namespaces, &facts).await?;
    }
    let addr = env::var("SUNGMA_ADDR").unwrap_or_else(|_| "127.0.0.1:8082".to_owned());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("sungma-write listening on {addr}");
    axum::serve(listener, router(Arc::new(AppState::new(interner, writer)))).await?;
    Ok(())
}
