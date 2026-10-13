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
    let namespaces = MemoryNamespaceStore::default();
    load_namespaces(&NAMESPACES, &dictionary, &namespaces).unwrap();
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

/// An identity is not interned, so it needs no lookup.
pub fn identity(identity: &str) -> Subject {
    Subject::Identity(identity.parse().unwrap())
}

/// The revision of the latest write.
pub async fn head(world: &World) -> Revision {
    world.facts.head().await.unwrap()
}

/// Declares `relation` in `namespace` with a rewrite that names its relations,
/// keeping the namespace's other relations. The namespace is checked whole, and
/// refused with its first problem, which names relations by id.
///
/// For the reader: each call stores a new namespace version, so relations
/// declared one at a time get epochs one revision apart. Once reads apply
/// epochs, a fact seeded before its relation's epoch silently stops
/// counting in the closure and decision tests. Declare each namespace
/// whole, or seed facts after every declaration.
pub fn declare(
    namespaces: &MemoryNamespaceStore,
    dictionary: &MemoryDictionary,
    namespace: &str,
    relation: &str,
    rewrite: Rewrite<&str>,
) -> Result<(), NamespaceError> {
    let namespace = NamespaceId(dictionary.intern(Pool::Namespaces, namespace));
    let relation = RelationId(dictionary.intern(Pool::Relations, relation));
    let rewrite = rewrite.map(&mut |name| RelationId(dictionary.intern(Pool::Relations, name)));
    let current = namespaces.namespace(namespace);
    let mut relations: Vec<_> = current
        .iter()
        .flat_map(|current| current.relations())
        .filter(|(declared, _)| **declared != relation)
        .map(|(declared, rewrite)| (*declared, rewrite.clone()))
        .collect();
    relations.push((relation, rewrite));
    let checked = Namespace::new(relations).map_err(|mut problems| problems.remove(0).error)?;
    namespaces.declare(namespace, checked).unwrap();
    Ok(())
}
