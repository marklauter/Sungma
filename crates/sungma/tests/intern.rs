//! Minting ids from leased blocks, with nodes simulated as interners
//! sharing one store.

use std::{
    collections::{HashMap, HashSet},
    num::NonZeroU32,
    ops::Range,
};

use proptest::{collection::vec, prelude::*, test_runner::TestCaseError};
use sungma::{
    intern::LeasingInterner,
    memory::MemoryDictionary,
    model::TheoryId,
    store::{
        Dictionary, InternError, Interner, NameStore,
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
    let error = node.intern(Identities, "alice").await.unwrap_err();
    assert!(
        matches!(error, InternError::Exhausted(Identities)),
        "{error}"
    );
}

#[tokio::test]
async fn setup_ids_and_leased_ids_never_meet() {
    let store = MemoryDictionary::default();
    let node = node(&store, 10);
    assert_eq!(store.intern(Identities, "a"), 0);
    assert_eq!(node.intern(Identities, "b").await.unwrap(), 1);
    assert_eq!(store.intern(Identities, "c"), 11);
}

/// `(node, pool, name)`: the node interns name `n{name}` in `POOLS[pool]`.
type Intern = (usize, usize, usize);

const POOLS: [Pool; 2] = [Identities, Pool::Resources(TheoryId(0))];

/// Nodes of the given block sizes, stale or not, intern names in turn.
async fn ids_are_unique_and_stable(
    nodes: Vec<(u32, bool)>,
    interns: Vec<Intern>,
) -> Result<(), TestCaseError> {
    let store = MemoryDictionary::default();
    let nodes: Vec<_> = nodes
        .into_iter()
        .map(|(size, stale)| {
            let node = Node {
                store: &store,
                stale,
                exhausted: false,
            };
            LeasingInterner::new(node, block(size))
        })
        .collect();
    let mut ids = HashMap::new();
    for (node, pool, name) in interns {
        let name = format!("n{name}");
        let node = &nodes[node % nodes.len()];
        let id = node.intern(POOLS[pool], &name).await.unwrap();
        let first = *ids.entry((pool, name.clone())).or_insert(id);
        prop_assert_eq!(id, first, "{} in {:?}", name, POOLS[pool]);
        prop_assert_eq!(store.name(POOLS[pool], id).await.unwrap(), Some(name));
    }
    let mut taken = HashSet::new();
    for ((pool, name), id) in ids {
        prop_assert!(taken.insert((pool, id)), "{name} shares {id}");
    }
    Ok(())
}

proptest! {
    /// However nodes interleave, a name keeps one id in its pool and no
    /// two names in a pool share one.
    #[test]
    fn a_name_keeps_one_id_across_nodes(
        nodes in vec((1..4u32, any::<bool>()), 1..4),
        interns in vec((0..4usize, 0..2usize, 0..6usize), 0..40),
    ) {
        let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
        runtime.block_on(ids_are_unique_and_stable(nodes, interns))?;
    }
}
