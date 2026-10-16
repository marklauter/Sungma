//! One node's contexts wired over one store edge: names and the interner,
//! the writer, Check's reader, and the API's reads, sharing the edge's
//! table. The node prints and parses namespace documents with
//! `sungma-lang` and injects both, so no context needs the document
//! language. [`Node::memory`] runs it on the memory edge, for tests and
//! development.

use std::{num::NonZeroU32, sync::Arc};

use sungma::{
    name::{NamespaceName, RelationName},
    namespace::Namespace,
    revision::Revision,
    store::{ReadingDictionary, StoreError},
};
use sungma_check::rows::RowReader;
use sungma_clock::{Clock, DevClock};
use sungma_edge::{
    memory::{MemorySequence, MemoryTable},
    read::{Consistency, ItemReader},
    sequence::Sequence,
    write::ItemWriter,
};
use sungma_lang::{fact, namespace};
use sungma_names::{read::NameRows, write::NameRowWriter};
use sungma_write::{
    intern::LeasingInterner,
    rows::RowWriter,
    service::{
        FactWriteService, FactWriteServiceError, NamespaceWriteService, NamespaceWriteServiceError,
    },
    store::NamespaceWrite,
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

pub type Names<E> = ReadingDictionary<NameRows<E>>;
pub type Interner<E, S> = LeasingInterner<NameRows<E>, NameRowWriter<E, E, S>>;
pub type Writer<E, C> = RowWriter<E, E, C, NameRows<E>, Print>;
pub type CheckReader<E> = RowReader<E, NameRows<E>, Parse>;

/// The contexts over the edge `E`, minting ids from the sequence `S`, and
/// stamping writes from the clock `C`.
pub struct Node<E, S, C = DevClock> {
    edge: E,
    names: Names<E>,
    interner: Interner<E, S>,
    writer: Writer<E, C>,
    check: CheckReader<E>,
}

impl Node<Arc<MemoryTable>, Arc<MemorySequence>> {
    /// A node on an empty memory edge.
    pub fn memory() -> Self {
        let edge = Arc::new(MemoryTable::new());
        Self::new(
            Arc::clone(&edge),
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
    pub async fn dev(edge: E, sequence: S) -> Result<Self, StoreError> {
        let head = sungma_schema::head::read(&edge, Consistency::Strong).await?;
        let clock = DevClock::starting_after(head.unwrap_or(Revision(0)));
        Ok(Self::new(edge, sequence, clock))
    }
}

impl<E, S, C> Node<E, S, C>
where
    E: ItemReader + ItemWriter + Clone,
{
    pub fn new(edge: E, sequence: S, clock: C) -> Self {
        let names = |consistency| NameRows::new(edge.clone(), consistency);
        Self {
            names: ReadingDictionary(names(Consistency::Eventual)),
            interner: LeasingInterner::new(
                names(Consistency::Strong),
                NameRowWriter::new(edge.clone(), edge.clone(), sequence),
                LEASE,
            ),
            writer: RowWriter::new(
                edge.clone(),
                edge.clone(),
                clock,
                names(Consistency::Strong),
                print as Print,
            ),
            check: RowReader::new(
                edge.clone(),
                names(Consistency::Eventual),
                parse as Parse,
                VERSIONS,
            ),
            edge,
        }
    }

    /// The edge, for the API's reads.
    pub fn edge(&self) -> &E {
        &self.edge
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
    /// Creates each namespace in the documents, then adds the facts in the
    /// fact document, through the write services, as a caller would. Every
    /// document is parsed before anything is written. Meant for an empty
    /// store: creating a namespace that has a version is refused.
    pub async fn seed(&self, namespaces: &[&str], facts: &str) -> Result<(), SeedError> {
        let documents = namespaces
            .iter()
            .map(|document| namespace::parse(document).map_err(namespace::Refusal))
            .collect::<Result<Vec<_>, _>>()?;
        let facts = fact::parse(facts)?;
        let writer = NamespaceWriteService::new(&self.interner, &self.writer);
        for document in documents {
            writer
                .write(
                    &document.namespace,
                    NamespaceWrite::Create(document.relations),
                )
                .await?;
        }
        if !facts.is_empty() {
            FactWriteService::new(&self.interner, &self.writer)
                .write(&facts, &[])
                .await?;
        }
        Ok(())
    }
}
