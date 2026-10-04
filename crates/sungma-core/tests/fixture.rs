//! Facts are loaded from JSON with one subject each.

use sungma_core::{
    fixture,
    memory::{MemoryDictionary, MemoryFactStore},
};

#[test]
fn a_fact_with_two_subjects_is_refused() {
    let mut dictionary = MemoryDictionary::default();
    let mut facts = MemoryFactStore::default();
    let json = r#"[{ "set": "file:a#owner", "identity": "alice", "resource": "folder:root" }]"#;
    let result = fixture::load_facts(json, &mut dictionary, &mut facts);
    assert!(result.is_err(), "expected an error, got {result:?}");
}
