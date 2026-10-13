//! The in-memory namespace store through its write port: a write is
//! refused unless the namespace is still at the version it was based on.

use std::sync::Arc;

use sungma::{
    id::{NamespaceId, RelationId},
    namespace::Namespace,
    revision::Revision,
    rewrite::Rewrite::{self, This},
    store::NamespaceStore,
};
use sungma_memory::MemoryNamespaceStore;
use sungma_write::store::{
    NamespaceWrite::{self, *},
    NamespaceWriteError, NamespaceWriter,
};

const FILE: NamespaceId = NamespaceId(0);
const OWNER: RelationId = RelationId(0);
const BANNED: RelationId = RelationId(1);

fn declaring(relations: &[RelationId]) -> Namespace {
    Namespace::new(relations.iter().map(|&relation| (relation, This)).collect()).unwrap()
}

async fn write(
    store: &MemoryNamespaceStore,
    write: NamespaceWrite,
) -> Result<Revision, NamespaceWriteError> {
    store.write_namespace(FILE, write).await
}

fn conflict(result: Result<Revision, NamespaceWriteError>) -> Option<Revision> {
    match result {
        Err(NamespaceWriteError::Conflict { current }) => current,
        other => panic!("expected a conflict, got {other:?}"),
    }
}

async fn rewrite(store: &MemoryNamespaceStore, relation: RelationId) -> Option<Arc<Rewrite>> {
    store.rewrite(FILE, relation).await.unwrap()
}

#[tokio::test]
async fn creating_a_namespace_that_has_a_current_version_is_refused() {
    let store = MemoryNamespaceStore::default();
    let created = write(&store, Create(declaring(&[OWNER]))).await.unwrap();
    assert_eq!(created, Revision(1));
    let refused = write(&store, Create(declaring(&[BANNED]))).await;
    assert_eq!(conflict(refused), Some(created));
    assert!(rewrite(&store, BANNED).await.is_none());
}

#[tokio::test]
async fn a_new_version_closes_the_current_one() {
    let store = MemoryNamespaceStore::default();
    let first = write(&store, Create(declaring(&[OWNER, BANNED])))
        .await
        .unwrap();
    let second = write(
        &store,
        NewVersion {
            based_on: first,
            namespace: declaring(&[OWNER]),
        },
    )
    .await
    .unwrap();
    assert_eq!(second, Revision(2));
    let current = store.version(FILE).unwrap();
    assert_eq!(current.revision(), second);
    assert_eq!(current.epoch(OWNER), Some(first));
    assert!(rewrite(&store, OWNER).await.is_some());
    assert!(rewrite(&store, BANNED).await.is_none());
}

#[tokio::test]
async fn a_write_based_on_an_older_version_stores_nothing() {
    let store = MemoryNamespaceStore::default();
    let first = write(&store, Create(declaring(&[OWNER]))).await.unwrap();
    let based_on = |namespace| NewVersion {
        based_on: first,
        namespace,
    };
    let second = write(&store, based_on(declaring(&[OWNER]))).await.unwrap();
    let refused = write(&store, based_on(declaring(&[BANNED]))).await;
    assert_eq!(conflict(refused), Some(second));
    let refused = write(&store, Retire { based_on: first }).await;
    assert_eq!(conflict(refused), Some(second));
    assert_eq!(store.version(FILE).unwrap().revision(), second);
    assert!(rewrite(&store, BANNED).await.is_none());
}

#[tokio::test]
async fn of_two_writes_based_on_one_version_exactly_one_succeeds() {
    let store = MemoryNamespaceStore::default();
    let first = write(&store, Create(declaring(&[OWNER]))).await.unwrap();
    let based_on = |relation| NewVersion {
        based_on: first,
        namespace: declaring(&[relation]),
    };
    let (left, right) = tokio::join!(
        write(&store, based_on(OWNER)),
        write(&store, based_on(BANNED))
    );
    assert_eq!(left.is_ok() as u8 + right.is_ok() as u8, 1);
}

#[tokio::test]
async fn a_retired_namespace_declares_nothing_until_it_is_created_again() {
    let store = MemoryNamespaceStore::default();
    let first = write(&store, Create(declaring(&[OWNER]))).await.unwrap();
    let retired = write(&store, Retire { based_on: first }).await.unwrap();
    assert!(store.version(FILE).is_none());
    assert!(rewrite(&store, OWNER).await.is_none());
    let refused = write(
        &store,
        NewVersion {
            based_on: first,
            namespace: declaring(&[OWNER]),
        },
    )
    .await;
    assert_eq!(conflict(refused), None);
    let refused = write(&store, Retire { based_on: first }).await;
    assert_eq!(conflict(refused), None);
    let again = write(&store, Create(declaring(&[OWNER]))).await.unwrap();
    assert!(again > retired);
    assert_eq!(store.version(FILE).unwrap().epoch(OWNER), Some(again));
}

#[test]
fn declaring_writes_the_next_version() {
    let mut store = MemoryNamespaceStore::default();
    store.declare(FILE, declaring(&[OWNER]));
    store.declare(FILE, declaring(&[OWNER, BANNED]));
    let current = store.version(FILE).unwrap();
    assert_eq!(current.revision(), Revision(2));
    assert_eq!(current.epoch(OWNER), Some(Revision(1)));
    assert_eq!(current.epoch(BANNED), Some(Revision(2)));
    assert!(store.namespace(FILE).unwrap().rewrite(&BANNED).is_some());
    assert!(store.namespace(NamespaceId(1)).is_none());
}

#[test]
fn a_conflict_says_the_namespace_moved_on() {
    let conflict = NamespaceWriteError::Conflict {
        current: Some(Revision(2)),
    };
    assert_eq!(
        conflict.to_string(),
        "the namespace isn't at the version the write was based on"
    );
}
