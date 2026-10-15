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
    store::{FactReader, Pool},
};
use sungma_check::resolve;
use sungma_lang::{fact::load_facts, namespace::load_namespaces};
use sungma_memory::{MemoryDictionary, MemoryStore};

pub const NAMESPACES: [&str; 3] = [
    include_str!("../fixtures/namespaces/file.json"),
    include_str!("../fixtures/namespaces/folder.json"),
    include_str!("../fixtures/namespaces/group.json"),
];

pub struct World {
    pub dictionary: MemoryDictionary,
    pub store: MemoryStore,
}

pub fn world() -> World {
    let dictionary = MemoryDictionary::default();
    let store = MemoryStore::default();
    load_namespaces(&NAMESPACES, &dictionary, &store).unwrap();
    load_facts(
        include_str!("../fixtures/docs.facts.json"),
        &dictionary,
        &store,
    )
    .unwrap();
    World { dictionary, store }
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
    world.store.head().await.unwrap()
}

/// Declares `relation` in `namespace` with a rewrite that names its relations,
/// keeping the namespace's other relations. The namespace is checked whole, and
/// refused with its first problem, which names relations by id.
///
/// Each call stores a new namespace version, so a relation it adds gets the
/// version's revision as its epoch, and facts seeded under that relation
/// before the call don't count. Seed facts after declaring their relations.
pub fn declare(
    namespaces: &MemoryStore,
    dictionary: &MemoryDictionary,
    namespace: &str,
    relation: &str,
    rewrite: Rewrite<&str>,
) -> Result<(), NamespaceError> {
    let intern = |pool, name| dictionary.intern(pool, name).expect("an id is left");
    let namespace = NamespaceId(intern(Pool::Namespaces, namespace));
    let relation = RelationId(intern(Pool::Relations, relation));
    let rewrite = rewrite.map(&mut |name| RelationId(intern(Pool::Relations, name)));
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
