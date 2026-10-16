//! The file, folder and group namespaces and their facts, on a memory
//! node. The namespaces are documents under `sungma/tests/fixtures/namespaces`,
//! and the facts are `sungma/tests/fixtures/docs.facts.json`.

// Each test file compiles this module separately and uses only part of it.
#![allow(dead_code)]

use sungma::{
    graph::{Subject, Subjectset},
    revision::Revision,
};
use sungma_names::resolve;

pub use sungma_node::testkit::World;

pub const NAMESPACES: [&str; 3] = [
    include_str!("../../../sungma/tests/fixtures/namespaces/file.json"),
    include_str!("../../../sungma/tests/fixtures/namespaces/folder.json"),
    include_str!("../../../sungma/tests/fixtures/namespaces/group.json"),
];

pub const FACTS: &str = include_str!("../../../sungma/tests/fixtures/docs.facts.json");

pub async fn world() -> World {
    World::load(&NAMESPACES, FACTS).await.unwrap()
}

/// Resolves `namespace:id#relation`; `None` if any name was never interned.
pub async fn subjectset(world: &World, text: &str) -> Option<Subjectset> {
    resolve::known(resolve::subjectset(world.names(), &text.parse().unwrap()).await).unwrap()
}

/// An identity is not interned, so it needs no lookup.
pub fn identity(identity: &str) -> Subject {
    Subject::Identity(identity.parse().unwrap())
}

/// The revision of the latest write.
pub async fn head(world: &World) -> Revision {
    world.head().await
}
