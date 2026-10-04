//! Minting ids from leased blocks, with nodes simulated as interners
//! sharing one store.

use std::{num::NonZeroU32, ops::Range};

use sungma_core::{
    intern::LeasingInterner,
    memory::MemoryDictionary,
    model::TheoryId,
    store::{
        Dictionary, Interner, NameStore,
        Pool::{self, Identities},
        StoreError,
    },
};

/// One node's view of a shared store. A stale node never finds a name, as
/// if another node's insert hadn't reached it; an exhausted node finds no
/// ids left to lease.
struct Node<'a> {
    store: &'a MemoryDictionary,
    stale: bool,
    exhausted: bool,
}

fn block(size: u32) -> NonZeroU32 {
    NonZeroU32::new(size).unwrap()
}

fn node(store: &MemoryDictionary, size: u32) -> LeasingInterner<Node<'_>> {
    LeasingInterner::new(
        Node {
            store,
            stale: false,
            exhausted: false,
        },
        block(size),
    )
}

impl Dictionary for Node<'_> {
    async fn lookup(&self, pool: Pool, name: &str) -> Result<Option<u32>, StoreError> {
        if self.stale {
            return Ok(None);
        }
        self.store.lookup(pool, name).await
    }

    async fn name(&self, pool: Pool, id: u32) -> Result<Option<String>, StoreError> {
        self.store.name(pool, id).await
    }
}

impl NameStore for Node<'_> {
    async fn insert_if_absent(&self, pool: Pool, name: &str, id: u32) -> Result<u32, StoreError> {
        self.store.insert_if_absent(pool, name, id).await
    }

    async fn lease(&self, pool: Pool, count: u32) -> Result<Range<u32>, StoreError> {
        if self.exhausted {
            return Ok(0..0);
        }
        self.store.lease(pool, count).await
    }
}

#[tokio::test]
async fn a_name_is_minted_once() {
    let store = MemoryDictionary::default();
    let node = node(&store, 10);
    let alice = node.intern(Identities, "alice").await.unwrap();
    let bob = node.intern(Identities, "bob").await.unwrap();
    assert_ne!(alice, bob);
    assert_eq!(node.intern(Identities, "alice").await.unwrap(), alice);
    assert_eq!(node.lookup(Identities, "alice").await.unwrap(), Some(alice));
    assert_eq!(node.lookup(Identities, "carol").await.unwrap(), None);
    assert_eq!(
        node.name(Identities, bob).await.unwrap().as_deref(),
        Some("bob")
    );
    assert_eq!(node.name(Identities, 99).await.unwrap(), None);
}

#[tokio::test]
async fn each_pool_mints_its_own_ids() {
    let store = MemoryDictionary::default();
    let node = node(&store, 10);
    let pools = [
        Pool::Theories,
        Pool::Relations,
        Pool::Resources(TheoryId(0)),
        Pool::Resources(TheoryId(1)),
        Identities,
    ];
    for pool in pools {
        assert_eq!(node.intern(pool, "x").await.unwrap(), 0, "{pool:?}");
    }
    for pool in pools {
        assert_eq!(node.intern(pool, &format!("{pool:?}")).await.unwrap(), 1);
    }
    for pool in pools {
        let name = format!("{pool:?}");
        assert_eq!(node.name(pool, 1).await.unwrap(), Some(name));
    }
}

#[tokio::test]
async fn nodes_mint_from_their_own_blocks() {
    let store = MemoryDictionary::default();
    let (a, b) = (node(&store, 2), node(&store, 2));
    let mut ids = Vec::new();
    for (node, name) in [(&a, "w"), (&a, "x"), (&b, "y"), (&a, "z")] {
        ids.push(node.intern(Identities, name).await.unwrap());
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
        block(10),
    );
    let first = a.intern(Identities, "alice").await.unwrap();
    // b misses a's insert, mints 10 from its own block, and loses.
    assert_eq!(b.intern(Identities, "alice").await.unwrap(), first);
    assert_eq!(store.name(Identities, 10).await.unwrap(), None);
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
        block(10),
    );
    assert!(node.intern(Identities, "alice").await.is_err());
}

#[tokio::test]
async fn setup_ids_and_leased_ids_never_meet() {
    let store = MemoryDictionary::default();
    let node = node(&store, 10);
    assert_eq!(store.intern(Identities, "a"), 0);
    assert_eq!(node.intern(Identities, "b").await.unwrap(), 1);
    assert_eq!(store.intern(Identities, "c"), 11);
}
