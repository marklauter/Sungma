//! Loading documents into the in-memory stores.

use sungma::store::Pool;
use sungma_lang::{
    fact::{FactError, load_facts},
    namespace::{LoadError, load_namespaces},
};
use sungma_memory::{MemoryDictionary, MemoryFactStore, MemoryNamespaceStore};
use sungma_write::store::{InternError, NameStore};

/// A dictionary with no ids left in `pool`.
async fn exhausted(pool: Pool) -> MemoryDictionary {
    let dictionary = MemoryDictionary::default();
    dictionary.lease(pool, u32::MAX).await.unwrap();
    dictionary
}

#[tokio::test]
async fn a_namespace_whose_names_find_no_ids_is_not_loaded() {
    let document = r#"{ "file": { "owner": "this", "viewer": "owner" } }"#;
    for pool in [Pool::Namespaces, Pool::Relations] {
        let dictionary = exhausted(pool).await;
        let namespaces = MemoryNamespaceStore::default();
        let loaded = load_namespaces(&[document], &dictionary, &namespaces);
        assert!(
            matches!(loaded, Err(LoadError::Intern(InternError::Exhausted(p))) if p == pool),
            "{loaded:?}"
        );
    }
}

#[tokio::test]
async fn a_fact_whose_names_find_no_ids_is_not_loaded() {
    let json = r#"[{ "namespace": "file", "resource": "a", "relation": "owner",
        "subject": { "identity": "alice" } }]"#;
    for pool in [Pool::Namespaces, Pool::Relations] {
        let dictionary = exhausted(pool).await;
        let loaded = load_facts(json, &dictionary, &MemoryFactStore::default());
        assert!(
            matches!(loaded, Err(FactError::Intern(InternError::Exhausted(p))) if p == pool),
            "{loaded:?}"
        );
    }
}
