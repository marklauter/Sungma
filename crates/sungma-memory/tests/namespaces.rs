//! The in-memory store's namespaces through the write port: a write is
//! refused unless the namespace is still at the version it was based on,
//! and a read at a revision sees the version whose span covers it.

use std::sync::Arc;

use sungma::{
    id::{NamespaceId, RelationId},
    namespace::Namespace,
    revision::Revision,
    rewrite::Rewrite::{self, This},
    store::{FactReader, NamespaceReader},
    version::{HistoryError, Version},
};
use sungma_memory::MemoryStore;
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
    store: &MemoryStore,
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

fn current(store: &MemoryStore) -> Option<Arc<Version>> {
    store.current(FILE)
}

async fn version_at(store: &MemoryStore, revision: Revision) -> Option<Arc<Version>> {
    store.version(FILE, revision).await.unwrap()
}

/// The relation's rewrite at the latest revision.
async fn rewrite(store: &MemoryStore, relation: RelationId) -> Option<Arc<Rewrite>> {
    let head = store.head().await.unwrap();
    let version = version_at(store, head).await?;
    version.namespace().rewrite(&relation).cloned()
}

#[tokio::test]
async fn creating_a_namespace_that_has_a_current_version_is_refused() {
    let store = MemoryStore::default();
    let created = write(&store, Create(declaring(&[OWNER]))).await.unwrap();
    assert_eq!(created, Revision(1));
    let refused = write(&store, Create(declaring(&[BANNED]))).await;
    assert_eq!(conflict(refused), Some(created));
    assert!(rewrite(&store, BANNED).await.is_none());
}

#[tokio::test]
async fn a_new_version_closes_the_current_one() {
    let store = MemoryStore::default();
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
    let current = current(&store).unwrap();
    assert_eq!(current.revision(), second);
    assert_eq!(current.epoch(OWNER), Some(first));
    assert!(rewrite(&store, OWNER).await.is_some());
    assert!(rewrite(&store, BANNED).await.is_none());
}

#[tokio::test]
async fn a_write_based_on_an_older_version_stores_nothing() {
    let store = MemoryStore::default();
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
    assert_eq!(current(&store).unwrap().revision(), second);
    assert!(rewrite(&store, BANNED).await.is_none());
}

/// Writers on separate threads, all based on one version, race for it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn of_writes_based_on_one_version_exactly_one_succeeds() {
    for _ in 0..50 {
        let store = Arc::new(MemoryStore::default());
        let first = write(&store, Create(declaring(&[OWNER]))).await.unwrap();
        let writers: Vec<_> = [OWNER, BANNED, OWNER, BANNED]
            .into_iter()
            .map(|relation| {
                let store = Arc::clone(&store);
                tokio::spawn(async move {
                    let write = NewVersion {
                        based_on: first,
                        namespace: declaring(&[relation]),
                    };
                    store.write_namespace(FILE, write).await.is_ok()
                })
            })
            .collect();
        let mut stored = 0;
        for writer in writers {
            stored += usize::from(writer.await.unwrap());
        }
        assert_eq!(stored, 1);
    }
}

#[tokio::test]
async fn a_retired_namespace_declares_nothing_until_it_is_created_again() {
    let store = MemoryStore::default();
    let first = write(&store, Create(declaring(&[OWNER]))).await.unwrap();
    let retired = write(&store, Retire { based_on: first }).await.unwrap();
    assert!(current(&store).is_none());
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
    assert!(matches!(
        refused,
        Err(NamespaceWriteError::History(HistoryError::NothingToRetire))
    ));
    let never = MemoryStore::default();
    let refused = write(&never, Retire { based_on: first }).await;
    assert!(matches!(
        refused,
        Err(NamespaceWriteError::History(HistoryError::NothingToRetire))
    ));
    let again = write(&store, Create(declaring(&[OWNER]))).await.unwrap();
    assert!(again > retired);
    assert_eq!(current(&store).unwrap().epoch(OWNER), Some(again));
}

#[tokio::test]
async fn a_read_between_two_versions_sees_the_earlier_one() {
    let store = MemoryStore::default();
    assert!(version_at(&store, Revision(0)).await.is_none());
    let first = write(&store, Create(declaring(&[OWNER]))).await.unwrap();
    store.declare(NamespaceId(1), declaring(&[])).unwrap();
    let second = write(
        &store,
        NewVersion {
            based_on: first,
            namespace: declaring(&[OWNER, BANNED]),
        },
    )
    .await
    .unwrap();
    let retired = write(&store, Retire { based_on: second }).await.unwrap();
    assert_eq!(
        [first, second, retired].map(|revision| revision.0),
        [1, 3, 4]
    );
    let opened = |version: Option<Arc<Version>>| version.map(|version| version.revision());
    assert_eq!(opened(version_at(&store, Revision(0)).await), None);
    assert_eq!(opened(version_at(&store, Revision(1)).await), Some(first));
    assert_eq!(opened(version_at(&store, Revision(2)).await), Some(first));
    assert_eq!(opened(version_at(&store, Revision(3)).await), Some(second));
    assert_eq!(opened(version_at(&store, Revision(4)).await), None);
    assert!(
        store
            .version(NamespaceId(2), Revision(4))
            .await
            .unwrap()
            .is_none()
    );
}

#[test]
fn declaring_writes_the_next_version() {
    let store = MemoryStore::default();
    assert_eq!(
        store.declare(FILE, declaring(&[OWNER])).unwrap(),
        Revision(1)
    );
    store.declare(FILE, declaring(&[OWNER, BANNED])).unwrap();
    let current = current(&store).unwrap();
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
