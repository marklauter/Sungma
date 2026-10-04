//! Facts are loaded from JSON with one subject each.

use sungma_core::{
    fixture,
    memory::{MemoryDictionary, MemoryFactStore},
    resolve,
    store::Pool,
};

#[test]
fn a_fact_with_two_subjects_is_refused() {
    let dictionary = MemoryDictionary::default();
    let facts = MemoryFactStore::default();
    let json = r#"[{ "set": "file:a#owner", "identity": "alice", "resource": "folder:root" }]"#;
    let result = fixture::load_facts(json, &dictionary, &facts);
    assert!(result.is_err(), "expected an error, got {result:?}");
}

#[tokio::test]
async fn resource_ids_are_pooled_per_theory() {
    let dictionary = MemoryDictionary::default();
    let facts = MemoryFactStore::default();
    let json = r#"[{ "set": "file:a#owner", "identity": "alice" }]"#;
    fixture::load_facts(json, &dictionary, &facts).unwrap();
    let file = resolve::resource(&dictionary, &"file:a".parse().unwrap()).await;
    assert!(file.unwrap().is_some());
    dictionary.intern(Pool::Theories, "folder");
    let folder = resolve::resource(&dictionary, &"folder:a".parse().unwrap()).await;
    assert_eq!(folder.unwrap(), None);
}
