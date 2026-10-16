//! Seeding a node from documents.

use std::sync::Arc;

use sungma::revision::Revision;
use sungma_check::store::FactReader;
use sungma_clock::DevClock;
use sungma_edge::{
    key::Key,
    memory::{MemorySequence, MemoryTable},
    sequence::Sequence,
};
use sungma_names::store::InternError;
use sungma_names::{resolve, store::Dictionary, store::Pool};
use sungma_node::{Node, SeedError, testkit::World};
use sungma_write::service::NamespaceWriteServiceError;

const FILE: &str = r#"{ "file": { "owner": "this", "viewer": "owner" } }"#;

/// A namespace whose names find no ids left is refused, and stores nothing.
#[tokio::test]
async fn a_namespace_whose_names_find_no_ids_is_not_seeded() {
    for (pool, tag) in [(Pool::Namespaces, 0u8), (Pool::Relations, 1u8)] {
        let sequence = Arc::new(MemorySequence::new());
        sequence
            .next(&Key::new((3u8, tag), 0u8), u64::MAX)
            .await
            .unwrap();
        let node = Node::new(Arc::new(MemoryTable::new()), sequence, DevClock::default());
        let seeded = node.seed(&[FILE], "[]").await;
        assert!(
            matches!(
                &seeded,
                Err(SeedError::Namespace(NamespaceWriteServiceError::Intern(
                    InternError::Exhausted(p)
                ))) if *p == pool
            ),
            "{seeded:?}"
        );
        assert_eq!(node.check().head().await.unwrap(), Revision(0));
    }
}

/// Every document is parsed before anything is written, so a refused one
/// stores nothing, not even a name.
#[tokio::test]
async fn a_refused_document_seeds_nothing() {
    let refused = [
        (vec![FILE, r#"{ "folder": { "viewer": "editor" } }"#], "[]"),
        (vec![FILE], "[{}]"),
    ];
    for (namespaces, facts) in refused {
        let world = World::new();
        assert!(world.node().seed(&namespaces, facts).await.is_err());
        assert_eq!(world.head().await, Revision(0));
        for (pool, name) in [(Pool::Namespaces, "file"), (Pool::Relations, "owner")] {
            assert_eq!(world.names().lookup(pool, name).await.unwrap(), None);
        }
    }
}

/// A fact the namespaces don't declare refuses the facts, after the
/// namespaces were written.
#[tokio::test]
async fn facts_the_namespaces_do_not_declare_are_refused() {
    let world = World::new();
    let facts = r#"[{ "namespace": "file", "resource": "a", "relation": "editor",
        "subject": { "identity": "alice" } }]"#;
    let seeded = world.node().seed(&[FILE], facts).await;
    assert!(matches!(seeded, Err(SeedError::Fact(_))), "{seeded:?}");
    let file = resolve::namespace(world.names(), &"file".parse().unwrap()).await;
    assert!(file.is_ok());
    assert_eq!(world.head().await, Revision(1));
}

/// A seed holds as many facts as it likes, whatever one write holds.
#[tokio::test]
async fn a_large_seed_is_written_in_parts() {
    let facts: Vec<String> = (0..250)
        .map(|i| {
            format!(
                r#"{{ "namespace": "file", "resource": "f{i}", "relation": "owner", "subject": {{ "identity": "p{i}" }} }}"#
            )
        })
        .collect();
    let world = World::load(&[FILE], &format!("[{}]", facts.join(",")))
        .await
        .unwrap();
    let set = resolve::subjectset(world.names(), &"file:f249#owner".parse().unwrap()).await;
    let head = world.head().await;
    let held = world.store().subjects(&set.unwrap(), head).await.unwrap();
    assert_eq!(held.len(), 1);
}
