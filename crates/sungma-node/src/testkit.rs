//! A node on the memory table, with the helpers tests reach for: seeding
//! documents, declaring a relation from a rewrite written with names, and
//! looking names up.

use std::sync::Arc;

use sungma::{
    id::{NamespaceId, RelationId},
    namespace::{Namespace, NamespaceError},
    revision::Revision,
    rewrite::Rewrite,
    version::Version,
};
use sungma_check::store::{FactReader, NamespaceReader};
use sungma_lang::fact;
use sungma_names::store::Interner;
use sungma_names::store::Pool;
use sungma_table::memory::{MemorySequence, MemoryTable};
use sungma_write::{
    service::FactWriteService,
    store::{NamespaceWrite, NamespaceWriter},
};

use crate::{CheckReader, Names, Node, SeedError};

type Memory = Arc<MemoryTable>;

/// A node on an empty memory table.
pub struct World {
    node: Node<Memory, Arc<MemorySequence>>,
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    pub fn new() -> Self {
        Self {
            node: Node::memory(),
        }
    }

    /// A world seeded with the documents.
    pub async fn load(namespaces: &[&str], facts: &str) -> Result<Self, SeedError> {
        let world = Self::new();
        world.node.seed(namespaces, facts).await?;
        Ok(world)
    }

    pub fn node(&self) -> &Node<Memory, Arc<MemorySequence>> {
        &self.node
    }

    /// Check's reads: facts and namespace versions.
    pub fn store(&self) -> &CheckReader<Memory> {
        self.node.check()
    }

    pub fn names(&self) -> &Names<Memory> {
        self.node.names()
    }

    /// The name's id in `pool`, minted if it's new there.
    pub async fn intern(&self, pool: Pool, name: &str) -> u32 {
        self.node.interner().intern(pool, name).await.unwrap()
    }

    /// Adds the facts in a fact document, in one write; an empty document
    /// writes nothing.
    pub async fn load_facts(&self, json: &str) -> Result<(), SeedError> {
        let facts = fact::parse(json)?;
        if facts.is_empty() {
            return Ok(());
        }
        FactWriteService::new(self.node.interner(), self.node.writer())
            .write(&facts, &[])
            .await?;
        Ok(())
    }

    /// Creates each namespace in the documents; see [`Node::seed`].
    pub async fn load_namespaces(&self, documents: &[&str]) -> Result<(), SeedError> {
        self.node.seed(documents, "[]").await
    }

    pub async fn head(&self) -> Revision {
        self.store().head().await.unwrap()
    }

    /// The namespace's version at the head, `None` when it has none.
    pub async fn latest(&self, namespace: NamespaceId) -> Option<Arc<Version>> {
        let head = self.head().await;
        self.store().version(namespace, head).await.unwrap()
    }

    /// Declares `relation` in `namespace` with a rewrite that names its
    /// relations, keeping the namespace's other relations, as a new
    /// version. The namespace is checked whole, and refused with its first
    /// problem, which names relations by id.
    ///
    /// Each call stores a new version, so a relation it adds gets the
    /// version's revision as its epoch, and facts seeded under it before
    /// the call don't count. Seed facts after declaring their relations.
    pub async fn declare(
        &self,
        namespace: &str,
        relation: &str,
        rewrite: Rewrite<&str>,
    ) -> Result<(), NamespaceError> {
        let namespace = NamespaceId(self.intern(Pool::Namespaces, namespace).await);
        let relation = RelationId(self.intern(Pool::Relations, relation).await);
        let mut names = Vec::new();
        rewrite.visit(&mut |name| names.push(*name));
        let mut ids = Vec::new();
        for name in names {
            ids.push((name, RelationId(self.intern(Pool::Relations, name).await)));
        }
        let rewrite = rewrite.map(&mut |name| {
            ids.iter()
                .find(|(named, _)| *named == name)
                .map(|(_, id)| *id)
                .unwrap_or(RelationId(u32::MAX))
        });
        let current = self.latest(namespace).await;
        let mut relations: Vec<_> = current
            .iter()
            .flat_map(|current| current.namespace().relations())
            .filter(|(declared, _)| **declared != relation)
            .map(|(declared, rewrite)| (*declared, rewrite.clone()))
            .collect();
        relations.push((relation, rewrite));
        let checked = Namespace::new(relations).map_err(|mut problems| problems.remove(0).error)?;
        let write = match current {
            Some(current) => NamespaceWrite::NewVersion {
                based_on: current.revision(),
                namespace: checked,
            },
            None => NamespaceWrite::Create(checked),
        };
        self.node
            .writer()
            .write_namespace(namespace, write)
            .await
            .unwrap();
        Ok(())
    }
}
