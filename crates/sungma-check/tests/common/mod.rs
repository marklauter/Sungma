//! The write and Check contexts over one edge, as the shared store tests
//! take them, and a table of its own for each DynamoDB test.

// Each test file compiles this module separately and uses only part of it.
#![allow(dead_code)]

use std::sync::atomic::{AtomicU64, Ordering};

use sungma::{
    name::{NamespaceName, RelationName},
    namespace::Namespace,
};
use sungma_check::rows::RowReader;
use sungma_clock::DevClock;
use sungma_dynamodb::{Table, local};
use sungma_edge::{
    read::{Consistency, ItemReader},
    sequence::Sequence,
    write::ItemWriter,
};
use sungma_lang::namespace;
use sungma_names::{read::NameRows, write::NameRowWriter};
use sungma_write::{rows::RowWriter, testkit::Stores};

pub type Print = fn(&NamespaceName, &Namespace<RelationName>) -> Result<String, String>;
pub type Parse = fn(&str) -> Result<(NamespaceName, Namespace<RelationName>), String>;

fn print(name: &NamespaceName, relations: &Namespace<RelationName>) -> Result<String, String> {
    namespace::print(name, relations).map_err(|error| error.to_string())
}

fn parse(document: &str) -> Result<(NamespaceName, Namespace<RelationName>), String> {
    namespace::parse(document)
        .map(|document| (document.namespace, document.relations))
        .map_err(|errors| namespace::Refusal(errors).to_string())
}

/// The contexts over one edge `E` and sequence `S`.
pub struct Contexts<E, S> {
    reader: RowReader<E, NameRows<E>, Parse>,
    writer: RowWriter<E, E, DevClock, NameRows<E>, Print>,
    names: NameRowWriter<E, E, S>,
}

impl<E: ItemReader + ItemWriter + Clone + Sync, S> Contexts<E, S> {
    pub async fn new(edge: E, sequence: S) -> Self {
        let names = |consistency| NameRows::new(edge.clone(), consistency);
        Self {
            reader: RowReader::new(edge.clone(), names(Consistency::Eventual), parse, 64),
            writer: RowWriter::dev(
                edge.clone(),
                edge.clone(),
                names(Consistency::Strong),
                print as Print,
            )
            .await
            .unwrap(),
            names: NameRowWriter::new(edge.clone(), edge, sequence),
        }
    }
}

impl<E, S> Stores for Contexts<E, S>
where
    E: ItemReader + ItemWriter + Send + Sync + 'static,
    S: Sequence + Send + Sync + 'static,
{
    type Reader = RowReader<E, NameRows<E>, Parse>;
    type Writer = RowWriter<E, E, DevClock, NameRows<E>, Print>;
    type Names = NameRowWriter<E, E, S>;

    fn reader(&self) -> &Self::Reader {
        &self.reader
    }

    fn writer(&self) -> &Self::Writer {
        &self.writer
    }

    fn names(&self) -> &Self::Names {
        &self.names
    }
}

/// A new table on DynamoDB Local at `SUNGMA_DYNAMODB_URL`,
/// `http://localhost:8000` by default.
pub async fn table() -> Table {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let url =
        std::env::var("SUNGMA_DYNAMODB_URL").unwrap_or_else(|_| "http://localhost:8000".to_owned());
    let name = format!(
        "sungma-check-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let table = Table::new(local(&url), name);
    table.create().await.unwrap_or_else(|error| {
        panic!("DynamoDB Local at {url} refused a table; run scripts/dynamodb.sh: {error}")
    });
    table
}
