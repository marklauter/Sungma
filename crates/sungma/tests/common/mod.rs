//! The file, folder and group theories and their facts, shared by the
//! integration tests. The theories are documents under
//! `fixtures/theories`, and the facts are `fixtures/docs.facts.json`.

// Each test file compiles this module separately and uses only part of it.
#![allow(dead_code)]

use sungma::{
    graph::{Subject, Subjectset},
    id::{RelationId, TheoryId},
    memory::{MemoryDictionary, MemoryFactStore, MemoryTheoryStore},
    resolve,
    revision::Revision,
    rewrite::Rewrite,
    store::{FactStore, Pool},
    theory::{Theory, TheoryError},
};
use sungma_lang::{fact::load_facts, theory::load_theories};

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
    load_theories(&THEORIES, &dictionary, &mut theories).unwrap();
    let facts = MemoryFactStore::default();
    load_facts(
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

/// Declares `relation` in `theory` with a rewrite that names its relations,
/// keeping the theory's other relations. The theory is checked whole, and
/// refused with its first problem, which names relations by id.
pub fn declare(
    theories: &mut MemoryTheoryStore,
    dictionary: &MemoryDictionary,
    theory: &str,
    relation: &str,
    rewrite: Rewrite<&str>,
) -> Result<(), TheoryError> {
    let theory = TheoryId(dictionary.intern(Pool::Theories, theory));
    let relation = RelationId(dictionary.intern(Pool::Relations, relation));
    let rewrite = rewrite.map(&mut |name| RelationId(dictionary.intern(Pool::Relations, name)));
    let mut relations: Vec<_> = theories
        .theory(theory)
        .into_iter()
        .flat_map(Theory::relations)
        .filter(|(declared, _)| **declared != relation)
        .map(|(declared, rewrite)| (*declared, rewrite.clone()))
        .collect();
    relations.push((relation, rewrite));
    let checked = Theory::new(relations).map_err(|mut problems| problems.remove(0).error)?;
    theories.declare(theory, checked);
    Ok(())
}
