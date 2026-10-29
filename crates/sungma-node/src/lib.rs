//! One node's contexts wired over one table: names and the interner,
//! the writer, Check's reader, and the API's reads, sharing the table's
//! table. The node prints and parses namespace documents with
//! `sungma-lang` and injects both, so no context needs the document
//! language. [`Node::memory`] runs it on the memory table, for tests and
//! development.

#[cfg(feature = "testkit")]
pub mod testkit;

use std::{num::NonZeroU32, sync::Arc};

use sungma::{
    name::{NamespaceName, RelationName},
    namespace::Namespace,
    revision::Revision,
    store::StoreError,
};
use sungma_check::rows::RowReader;
use sungma_clock::{Clock, DevClock};
use sungma_lang::{fact, namespace};
use sungma_names::intern::LeasingInterner;
use sungma_names::store::ReadingDictionary;
use sungma_names::{read::NameRows, write::NameRowWriter};
use sungma_table::{
    memory::{MemorySequence, MemoryTable},
    read::{Consistency, ItemReader},
    sequence::Sequence,
    write::ItemWriter,
};
use sungma_write::{
    rows::RowWriter,
    service::{self, FactWriteServiceError, NamespaceWriteServiceError},
};
use thiserror::Error;

/// How many ids a lease takes at a time.
const LEASE: NonZeroU32 = NonZeroU32::new(64).unwrap();

/// How many namespace versions Check keeps parsed.
const VERSIONS: usize = 1024;

pub type Print = fn(&NamespaceName, &Namespace<RelationName>) -> Result<String, String>;
pub type Parse = fn(&str) -> Result<(NamespaceName, Namespace<RelationName>), String>;

/// The printer's canonical document.
fn print(name: &NamespaceName, relations: &Namespace<RelationName>) -> Result<String, String> {
    namespace::print(name, relations).map_err(|error| error.to_string())
}

fn parse(document: &str) -> Result<(NamespaceName, Namespace<RelationName>), String> {
    namespace::parse(document)
        .map(|document| (document.namespace, document.relations))
        .map_err(|errors| namespace::Refusal(errors).to_string())
}

/// Why seed documents didn't load.
#[derive(Debug, Error)]
pub enum SeedError {
    #[error(transparent)]
    Refused(#[from] namespace::Refusal),
    #[error(transparent)]
    Namespace(#[from] NamespaceWriteServiceError),
    #[error(transparent)]
    Facts(#[from] serde_json::Error),
    #[error(transparent)]
    Fact(#[from] FactWriteServiceError),
}

impl From<service::SeedError> for SeedError {
    fn from(error: service::SeedError) -> Self {
        match error {
            service::SeedError::Namespace(error) => Self::Namespace(error),
            service::SeedError::Fact(error) => Self::Fact(error),
        }
    }
}

pub type Names<E> = ReadingDictionary<NameRows<E>>;
pub type Interner<E, S> = LeasingInterner<NameRows<E>, NameRowWriter<E, E, S>>;
pub type Writer<E, C> = RowWriter<E, E, C, NameRows<E>, Print>;
pub type CheckReader<E> = RowReader<E, NameRows<E>, Parse>;

/// Names read eventually, for a read that resolves names.
pub fn names<E: Clone>(table: &E) -> Names<E> {
    ReadingDictionary(NameRows::new(table.clone(), Consistency::Eventual))
}

/// Names read strongly that mints new ones from `sequence`, for a write.
pub fn interner<E: Clone, S>(table: &E, sequence: S) -> Interner<E, S> {
    LeasingInterner::new(
        NameRows::new(table.clone(), Consistency::Strong),
        NameRowWriter::new(table.clone(), table.clone(), sequence),
        LEASE,
    )
}

/// The writer, stamping writes from `clock`.
pub fn writer<E: Clone, C>(table: &E, clock: C) -> Writer<E, C> {
    RowWriter::new(
        table.clone(),
        table.clone(),
        clock,
        NameRows::new(table.clone(), Consistency::Strong),
        print as Print,
    )
}

/// Check's facts and buffered namespace versions.
pub fn check_reader<E: Clone>(table: &E) -> CheckReader<E> {
    RowReader::new(
        table.clone(),
        NameRows::new(table.clone(), Consistency::Eventual),
        parse as Parse,
        VERSIONS,
    )
}

/// The contexts over the table `E`, minting ids from the sequence `S`, and
/// stamping writes from the clock `C`.
pub struct Node<E, S, C = DevClock> {
    table: E,
    names: Names<E>,
    interner: Interner<E, S>,
    writer: Writer<E, C>,
    check: CheckReader<E>,
}

impl Node<Arc<MemoryTable>, Arc<MemorySequence>> {
    /// A node on an empty memory table.
    pub fn memory() -> Self {
        let table = Arc::new(MemoryTable::new());
        Self::new(
            Arc::clone(&table),
            Arc::new(MemorySequence::new()),
            DevClock::default(),
        )
    }
}

impl<E, S> Node<E, S>
where
    E: ItemReader + ItemWriter + Clone + Sync,
{
    /// A node whose [`DevClock`] starts after the head it finds, so a
    /// restarted node doesn't reuse a revision already committed.
    pub async fn dev(table: E, sequence: S) -> Result<Self, StoreError> {
        let head = sungma_schema::head::read(&table, Consistency::Strong).await?;
        let clock = DevClock::starting_after(head.unwrap_or(Revision(0)));
        Ok(Self::new(table, sequence, clock))
    }
}

impl<E, S, C> Node<E, S, C>
where
    E: ItemReader + ItemWriter + Clone,
{
    pub fn new(table: E, sequence: S, clock: C) -> Self {
        Self {
            names: names(&table),
            interner: interner(&table, sequence),
            writer: writer(&table, clock),
            check: check_reader(&table),
            table,
        }
    }

    /// The table, for the API's reads.
    pub fn table(&self) -> &E {
        &self.table
    }

    /// Names read eventually, for a read that resolves names.
    pub fn names(&self) -> &Names<E> {
        &self.names
    }

    /// Names read strongly that mints new ones, for a write.
    pub fn interner(&self) -> &Interner<E, S> {
        &self.interner
    }

    pub fn writer(&self) -> &Writer<E, C> {
        &self.writer
    }

    /// Check's facts and buffered namespace versions.
    pub fn check(&self) -> &CheckReader<E> {
        &self.check
    }
}

impl<E, S, C> Node<E, S, C>
where
    E: ItemReader + ItemWriter + Clone + Send + Sync,
    S: Sequence + Send + Sync,
    C: Clock + Sync,
{
    /// Seeds the namespace documents and the fact document; see
    /// [`service::seed`]. Every document is parsed before anything is
    /// written.
    pub async fn seed(&self, namespaces: &[&str], facts: &str) -> Result<(), SeedError> {
        let documents = namespaces
            .iter()
            .map(|document| namespace::parse(document).map_err(namespace::Refusal))
            .collect::<Result<Vec<_>, _>>()?;
        let facts = fact::parse(facts)?;
        let namespaces = documents
            .into_iter()
            .map(|document| (document.namespace, document.relations))
            .collect();
        service::seed(&self.interner, &self.writer, namespaces, &facts).await?;
        Ok(())
    }
}
