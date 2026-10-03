//! The file, folder and group theories and their facts, shared by the
//! integration tests:
//!
//! ```yaml
//! file:
//!   - owner
//!   - parent
//!   - editor: this | owner
//!   - viewer: (this | editor | (parent, viewer)) ! banned
//!   - auditor: this & viewer
//!   - banned
//! folder:
//!   - owner
//!   - parent
//!   - viewer: (this | (parent, viewer)) ! banned
//!   - banned
//! group:
//!   - member
//! ```

// Each test file compiles this module separately and uses only part of it.
#![allow(dead_code)]

use sungma_core::{
    fixture,
    memory::{MemoryDictionary, MemoryFactStore},
    model::{Revision, Subject, Subjectset},
    resolve,
    rewrite::Rewrite::{self, Computed, Exclusion, FactTo, Intersection, This, Union},
    store::FactStore,
    theory::Theories,
};

pub struct World {
    pub dictionary: MemoryDictionary,
    pub theories: Theories,
    pub facts: MemoryFactStore,
}

pub fn world() -> World {
    let mut dictionary = MemoryDictionary::default();
    let mut theories = Theories::default();
    let viewable = |base: Vec<Rewrite<&'static str>>| {
        Exclusion(Box::new(Union(base)), Box::new(Computed("banned")))
    };
    let parent_viewer = || FactTo {
        factset: "parent",
        computed: "viewer",
    };

    let relations = [
        ("file", "owner", This),
        ("file", "parent", This),
        ("file", "editor", Union(vec![This, Computed("owner")])),
        (
            "file",
            "viewer",
            viewable(vec![This, Computed("editor"), parent_viewer()]),
        ),
        (
            "file",
            "auditor",
            Intersection(vec![This, Computed("viewer")]),
        ),
        ("file", "banned", This),
        ("folder", "owner", This),
        ("folder", "parent", This),
        ("folder", "viewer", viewable(vec![This, parent_viewer()])),
        ("folder", "banned", This),
        ("group", "member", This),
    ];
    for (theory, relation, rewrite) in relations {
        fixture::declare(&mut theories, &mut dictionary, theory, relation, rewrite).unwrap();
    }

    let mut facts = MemoryFactStore::default();
    fixture::load_facts(
        include_str!("../fixtures/docs.json"),
        &mut dictionary,
        &mut facts,
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
    let (resource, relation) = text.rsplit_once('#').unwrap();
    let (theory, id) = resource.split_once(':').unwrap();
    resolve::subjectset(&world.dictionary, theory, id, relation)
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
