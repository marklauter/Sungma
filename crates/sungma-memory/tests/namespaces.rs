//! What only the in-memory store does with namespaces: `declare` writes
//! the next version whatever is current. The tests every store runs are in
//! `stores.rs`.

use sungma::{
    id::{NamespaceId, RelationId},
    namespace::Namespace,
    revision::Revision,
    rewrite::Rewrite::This,
};
use sungma_memory::MemoryStore;
use sungma_write::store::NamespaceWriteError;

const FILE: NamespaceId = NamespaceId(0);
const OWNER: RelationId = RelationId(0);
const BANNED: RelationId = RelationId(1);

fn declaring(relations: &[RelationId]) -> Namespace {
    Namespace::new(relations.iter().map(|&relation| (relation, This)).collect()).unwrap()
}

#[test]
fn declaring_writes_the_next_version() {
    let store = MemoryStore::default();
    assert_eq!(
        store.declare(FILE, declaring(&[OWNER])).unwrap(),
        Revision(1)
    );
    store.declare(FILE, declaring(&[OWNER, BANNED])).unwrap();
    let current = store.current(FILE).unwrap();
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
