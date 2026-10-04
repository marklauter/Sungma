//! Minting ids from leased blocks, with nodes simulated as interners
//! sharing one store.

use std::ops::Range;

use sungma_core::{
    intern::LeasingInterner,
    memory::MemoryDictionary,
    store::{Dictionary, Interner, NameStore, StoreError},
};

/// One node's view of a shared store. A stale node never finds a name, as
/// if another node's insert hadn't reached it; an exhausted node finds no
/// ids left to lease.
struct Node<'a> {
    store: &'a MemoryDictionary,
    stale: bool,
    exhausted: bool,
}

fn node(store: &MemoryDictionary, block: u32) -> LeasingInterner<Node<'_>> {
    LeasingInterner::new(
        Node {
            store,
            stale: false,
            exhausted: false,
        },
        block,
    )
}

impl Dictionary for Node<'_> {
    async fn lookup(&self, name: &str) -> Result<Option<u32>, StoreError> {
        if self.stale {
            return Ok(None);
        }
        self.store.lookup(name).await
    }

    async fn name(&self, id: u32) -> Result<Option<String>, StoreError> {
        self.store.name(id).await
    }
}

impl NameStore for Node<'_> {
    async fn insert_if_absent(&self, name: &str, id: u32) -> Result<u32, StoreError> {
        self.store.insert_if_absent(name, id).await
    }

    async fn lease(&self, count: u32) -> Result<Range<u32>, StoreError> {
        if self.exhausted {
            return Ok(0..0);
        }
        self.store.lease(count).await
    }
}

#[tokio::test]
async fn a_name_is_minted_once() {
    let store = MemoryDictionary::default();
    let node = node(&store, 10);
    let alice = node.intern("alice").await.unwrap();
    let bob = node.intern("bob").await.unwrap();
    assert_ne!(alice, bob);
    assert_eq!(node.intern("alice").await.unwrap(), alice);
    assert_eq!(node.lookup("alice").await.unwrap(), Some(alice));
    assert_eq!(node.lookup("carol").await.unwrap(), None);
    assert_eq!(node.name(bob).await.unwrap().as_deref(), Some("bob"));
    assert_eq!(node.name(99).await.unwrap(), None);
}

#[tokio::test]
async fn nodes_mint_from_their_own_blocks() {
    let store = MemoryDictionary::default();
    let (a, b) = (node(&store, 2), node(&store, 2));
    let mut ids = Vec::new();
    for (node, name) in [(&a, "w"), (&a, "x"), (&b, "y"), (&a, "z")] {
        ids.push(node.intern(name).await.unwrap());
    }
    assert_eq!(ids, [0, 1, 2, 4]);
}

#[tokio::test]
async fn the_first_insert_of_a_name_wins() {
    let store = MemoryDictionary::default();
    let a = node(&store, 10);
    let b = LeasingInterner::new(
        Node {
            store: &store,
            stale: true,
            exhausted: false,
        },
        10,
    );
    let first = a.intern("alice").await.unwrap();
    // b misses a's insert, mints 10 from its own block, and loses.
    assert_eq!(b.intern("alice").await.unwrap(), first);
    assert_eq!(store.name(10).await.unwrap(), None);
}

#[tokio::test]
async fn interning_fails_once_the_ids_run_out() {
    let store = MemoryDictionary::default();
    let node = LeasingInterner::new(
        Node {
            store: &store,
            stale: false,
            exhausted: true,
        },
        10,
    );
    assert!(node.intern("alice").await.is_err());
}

#[tokio::test]
async fn setup_ids_and_leased_ids_never_meet() {
    let store = MemoryDictionary::default();
    let node = node(&store, 10);
    assert_eq!(store.intern("a"), 0);
    assert_eq!(node.intern("b").await.unwrap(), 1);
    assert_eq!(store.intern("c"), 11);
}
