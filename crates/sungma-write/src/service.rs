//! The write services: take a write as a caller names it, resolve its
//! names, and apply it through the writer ports.

use std::collections::HashMap;

use thiserror::Error;

use sungma::{
    graph::Fact,
    id::{NamespaceId, RelationId},
    name::{FactName, NamespaceName, RelationName},
    namespace::{Namespace, NamespaceError},
    revision::Revision,
    store::StoreError,
};
use sungma_clock::ClockFault;
use sungma_names::{
    resolve::{self, ResolveError, UnknownName},
    store::Dictionary,
    store::Pool,
};

use sungma_names::store::{InternError, Interner};

use crate::store::{
    FactWrite, FactWriteError, FactWriter, NamespaceWrite, NamespaceWriteError, NamespaceWriter,
};

#[derive(Debug, Error)]
pub enum FactWriteServiceError {
    #[error(transparent)]
    Unknown(UnknownName),
    /// The latest namespace versions don't declare the fact's relation, or
    /// its subjectset subject's relation.
    #[error("'{0}' names a relation the latest namespace version doesn't declare")]
    Undeclared(FactName),
    /// More facts than one write holds; the caller splits them.
    #[error("a write of {rows} rows is more than one write holds, {most}")]
    TooLarge { rows: usize, most: usize },
    #[error(transparent)]
    Clock(ClockFault),
    #[error(transparent)]
    Store(StoreError),
}

impl From<ResolveError> for FactWriteServiceError {
    fn from(error: ResolveError) -> Self {
        match error {
            ResolveError::Unknown(name) => Self::Unknown(name),
            ResolveError::Store(error) => Self::Store(error),
        }
    }
}

/// Writes facts named as a caller names them. Names are looked up, never
/// interned: a fact can only name what a namespace declared.
pub struct FactWriteService<'a, D, W> {
    dictionary: &'a D,
    facts: &'a W,
}

impl<'a, D, W> FactWriteService<'a, D, W>
where
    D: Dictionary + Sync,
    W: FactWriter + Sync,
{
    pub fn new(dictionary: &'a D, facts: &'a W) -> Self {
        Self { dictionary, facts }
    }

    /// Retires `retire` and adds `add` at one revision, and returns it. The
    /// retirements apply first, so a fact in both lists is held after the
    /// write. A write naming a name never interned, or adding a fact the
    /// latest versions don't declare, is refused whole.
    pub async fn write(
        &self,
        add: &[FactName],
        retire: &[FactName],
    ) -> Result<Revision, FactWriteServiceError> {
        let mut writes = Vec::with_capacity(add.len() + retire.len());
        for name in retire {
            writes.push(FactWrite::Retire(self.fact(name).await?));
        }
        for name in add {
            writes.push(FactWrite::Insert(self.fact(name).await?));
        }
        // The additions follow the retirements, in the caller's order.
        let added = |fact: &Fact| {
            let at = writes[retire.len()..]
                .iter()
                .position(|write| matches!(write, FactWrite::Insert(f) if f == fact))?;
            add.get(at)
        };
        self.facts
            .write(&writes)
            .await
            .map_err(|error| match error {
                FactWriteError::Undeclared(fact) => match added(&fact) {
                    Some(name) => FactWriteServiceError::Undeclared(name.clone()),
                    None => FactWriteServiceError::Store(StoreError(format!(
                        "the store refused a fact it wasn't given to add: {fact:?}"
                    ))),
                },
                FactWriteError::TooLarge { rows, most } => {
                    FactWriteServiceError::TooLarge { rows, most }
                }
                FactWriteError::Clock(fault) => FactWriteServiceError::Clock(fault),
                FactWriteError::Store(error) => FactWriteServiceError::Store(error),
            })
    }

    async fn fact(&self, name: &FactName) -> Result<Fact, ResolveError> {
        resolve::fact(self.dictionary, name).await
    }
}

#[derive(Debug, Error)]
pub enum NamespaceWriteServiceError {
    #[error(transparent)]
    Intern(#[from] InternError),
    #[error(transparent)]
    Write(#[from] NamespaceWriteError),
}

/// Interning distinct names gives distinct ids, so a rename to them can't
/// give two relations one id; a store that broke that has failed.
impl From<NamespaceError> for NamespaceWriteServiceError {
    fn from(error: NamespaceError) -> Self {
        InternError::Store(StoreError(error.to_string())).into()
    }
}

/// Writes namespace versions named as a caller names them. A version's
/// names are interned before it is written, so a write refused after that,
/// by a conflict, leaves its names interned. An interned name changes
/// nothing a read sees.
pub struct NamespaceWriteService<'a, I, W> {
    interner: &'a I,
    namespaces: &'a W,
}

impl<'a, I, W> NamespaceWriteService<'a, I, W>
where
    I: Interner + Sync,
    W: NamespaceWriter + Sync,
{
    pub fn new(interner: &'a I, namespaces: &'a W) -> Self {
        Self {
            interner,
            namespaces,
        }
    }

    /// Applies `write` to the namespace `name` and returns its revision.
    /// Retiring a namespace never interned is refused as retiring one with
    /// no latest version, and interns nothing.
    pub async fn write(
        &self,
        name: &NamespaceName,
        write: NamespaceWrite<RelationName>,
    ) -> Result<Revision, NamespaceWriteServiceError> {
        let (id, write) = match write {
            NamespaceWrite::Retire { based_on } => {
                let id = self
                    .interner
                    .lookup(Pool::Namespaces, name.as_str())
                    .await
                    .map_err(InternError::from)?
                    .ok_or(NamespaceWriteError::NothingToRetire)?;
                (NamespaceId(id), NamespaceWrite::Retire { based_on })
            }
            NamespaceWrite::Create(namespace) => (
                self.namespace_id(name).await?,
                NamespaceWrite::Create(self.intern(&namespace).await?),
            ),
            NamespaceWrite::NewVersion {
                based_on,
                namespace,
            } => (
                self.namespace_id(name).await?,
                NamespaceWrite::NewVersion {
                    based_on,
                    namespace: self.intern(&namespace).await?,
                },
            ),
        };
        Ok(self.namespaces.write_namespace(id, write).await?)
    }

    async fn namespace_id(&self, name: &NamespaceName) -> Result<NamespaceId, InternError> {
        Ok(NamespaceId(
            self.interner
                .intern(Pool::Namespaces, name.as_str())
                .await?,
        ))
    }

    /// The namespace with each relation name interned.
    async fn intern(
        &self,
        namespace: &Namespace<RelationName>,
    ) -> Result<Namespace, NamespaceWriteServiceError> {
        let mut ids = HashMap::new();
        for name in namespace.names() {
            let id = self.interner.intern(Pool::Relations, name.as_str()).await?;
            ids.insert(name.clone(), RelationId(id));
        }
        namespace.try_map(|name| {
            ids.get(name).copied().ok_or_else(|| {
                InternError::Store(StoreError(format!("relation '{name}' wasn't interned"))).into()
            })
        })
    }
}

/// How many facts a seed writes at a time. A new fact takes two rows and
/// may name two namespaces, so this many fit one write whatever they are.
const SEED_FACTS: usize = 24;

/// Why a seed didn't load.
#[derive(Debug, Error)]
pub enum SeedError {
    #[error(transparent)]
    Namespace(#[from] NamespaceWriteServiceError),
    #[error(transparent)]
    Fact(#[from] FactWriteServiceError),
}

/// Creates each namespace, then adds the facts, through the write
/// services, as a caller would, in as many writes as the facts take.
/// Meant for an empty store: creating a namespace that has a version is
/// refused.
pub async fn seed<I, W>(
    interner: &I,
    writer: &W,
    namespaces: Vec<(NamespaceName, Namespace<RelationName>)>,
    facts: &[FactName],
) -> Result<(), SeedError>
where
    I: Interner + Sync,
    W: NamespaceWriter + FactWriter + Sync,
{
    let namespace_writer = NamespaceWriteService::new(interner, writer);
    for (name, relations) in namespaces {
        namespace_writer
            .write(&name, NamespaceWrite::Create(relations))
            .await?;
    }
    let fact_writer = FactWriteService::new(interner, writer);
    for facts in facts.chunks(SEED_FACTS) {
        fact_writer.write(facts, &[]).await?;
    }
    Ok(())
}
