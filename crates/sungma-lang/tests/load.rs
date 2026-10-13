//! Loading documents into the in-memory stores.

use sungma::{
    revision::Revision,
    store::{Dictionary, FactStore, Pool},
};
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

#[tokio::test]
async fn a_fact_document_that_runs_out_of_ids_part_way_stores_no_fact() {
    let dictionary = MemoryDictionary::default();
    dictionary
        .lease(Pool::Relations, u32::MAX - 1)
        .await
        .unwrap();
    let json = r#"[
        { "namespace": "file", "resource": "a", "relation": "owner",
          "subject": { "identity": "alice" } },
        { "namespace": "file", "resource": "a", "relation": "viewer",
          "subject": { "identity": "bob" } }
    ]"#;
    let facts = MemoryFactStore::default();
    let loaded = load_facts(json, &dictionary, &facts);
    assert!(matches!(loaded, Err(FactError::Intern(_))), "{loaded:?}");
    assert_eq!(facts.head().await.unwrap(), Revision(0));
}

#[tokio::test]
async fn a_refused_namespace_document_interns_no_name() {
    let dictionary = MemoryDictionary::default();
    let documents = [
        r#"{ "file": { "owner": "this" } }"#,
        r#"{ "folder": { "viewer": "editor" } }"#,
    ];
    let loaded = load_namespaces(&documents, &dictionary, &MemoryNamespaceStore::default());
    assert!(matches!(loaded, Err(LoadError::Refused(_))), "{loaded:?}");
    for (pool, name) in [(Pool::Namespaces, "file"), (Pool::Relations, "owner")] {
        assert_eq!(dictionary.lookup(pool, name).await.unwrap(), None);
    }
}
