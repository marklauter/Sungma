//! The file, folder and group theories and their facts, shared by the
//! integration tests. The theories are documents under
//! `fixtures/theories`, and the facts are `fixtures/docs.facts.json`.

// Each test file compiles this module separately and uses only part of it.
#![allow(dead_code)]

use sungma_core::{
    fixture,
    memory::{MemoryDictionary, MemoryFactStore, MemoryTheoryStore},
    model::{Revision, Subject, Subjectset},
    resolve,
    store::FactStore,
};

pub const THEORIES: [&str; 3] = [
    include_str!("../fixtures/theories/file.json"),
    include_str!("../fixtures/theories/folder.json"),
    include_str!("../fixtures/theories/group.json"),
];

pub struct World {
    pub dictionary: MemoryDictionary,
    pub theories: MemoryTheoryStore,
    pub facts: MemoryFactStore,
}

pub fn world() -> World {
    let dictionary = MemoryDictionary::default();
    let mut theories = MemoryTheoryStore::default();
    for document in THEORIES {
        fixture::load_theory(document, &dictionary, &mut theories).unwrap();
    }
    let facts = MemoryFactStore::default();
    fixture::load_facts(
        include_str!("../fixtures/docs.facts.json"),
        &dictionary,
        &facts,
    )
    .unwrap();
    World {
        dictionary,
        theories,
        facts,
    }
}

/// Resolves `theory:id#relation`; `None` if any name was never interned.
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
