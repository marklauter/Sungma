//! The file, folder and group namespaces and their facts, shared by the
//! integration tests. The namespaces are documents under
//! `fixtures/namespaces`, and the facts are `fixtures/docs.facts.json`.

// Each test file compiles this module separately and uses only part of it.
#![allow(dead_code)]

use sungma::{
    graph::{Subject, Subjectset},
    id::{NamespaceId, RelationId},
    namespace::{Namespace, NamespaceError},
    revision::Revision,
    rewrite::Rewrite,
    store::{FactStore, Pool},
};
use sungma_check::resolve;
use sungma_lang::{fact::load_facts, namespace::load_namespaces};
use sungma_memory::{MemoryDictionary, MemoryFactStore, MemoryNamespaceStore};

pub const NAMESPACES: [&str; 3] = [
    include_str!("../fixtures/namespaces/file.json"),
    include_str!("../fixtures/namespaces/folder.json"),
    include_str!("../fixtures/namespaces/group.json"),
];

pub struct World {
    pub dictionary: MemoryDictionary,
    pub namespaces: MemoryNamespaceStore,
    pub facts: MemoryFactStore,
}

pub fn world() -> World {
    let dictionary = MemoryDictionary::default();
    let mut namespaces = MemoryNamespaceStore::default();
    load_namespaces(&NAMESPACES, &dictionary, &mut namespaces).unwrap();
    let facts = MemoryFactStore::default();
    load_facts(
        include_str!("../fixtures/docs.facts.json"),
        &dictionary,
        &facts,
    )
    .unwrap();
    World {
        dictionary,
        namespaces,
        facts,
    }
}

/// Resolves `namespace:id#relation`; `None` if any name was never interned.
pub async fn subjectset(world: &World, text: &str) -> Option<Subjectset> {
    resolve::subjectset(&world.dictionary, &text.parse().unwrap())
        .await
        .unwrap()
}

pub async fn identity(world: &World, identity: &str) -> Option<Subject> {
    let identity = resolve::identity(&world.dictionary, identity)
        .await
        .unwrap();
    identity.map(Subject::Identity)
}

/// The revision of the latest write.
pub async fn head(world: &World) -> Revision {
    world.facts.head().await.unwrap()
}

/// Declares `relation` in `namespace` with a rewrite that names its relations,
/// keeping the namespace's other relations. The namespace is checked whole, and
/// refused with its first problem, which names relations by id.
pub fn declare(
    namespaces: &mut MemoryNamespaceStore,
    dictionary: &MemoryDictionary,
    namespace: &str,
    relation: &str,
    rewrite: Rewrite<&str>,
) -> Result<(), NamespaceError> {
    let namespace = NamespaceId(dictionary.intern(Pool::Namespaces, namespace));
    let relation = RelationId(dictionary.intern(Pool::Relations, relation));
    let rewrite = rewrite.map(&mut |name| RelationId(dictionary.intern(Pool::Relations, name)));
    let mut relations: Vec<_> = namespaces
        .namespace(namespace)
        .into_iter()
        .flat_map(Namespace::relations)
        .filter(|(declared, _)| **declared != relation)
        .map(|(declared, rewrite)| (*declared, rewrite.clone()))
        .collect();
    relations.push((relation, rewrite));
    let checked = Namespace::new(relations).map_err(|mut problems| problems.remove(0).error)?;
    namespaces.declare(namespace, checked);
    Ok(())
}
