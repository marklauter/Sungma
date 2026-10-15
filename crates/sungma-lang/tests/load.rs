//! Loading documents into the in-memory stores.

use sungma::{
    revision::Revision,
    store::{Dictionary, FactStore, Pool},
};
use sungma_lang::{
    fact::{FactError, load_facts},
    namespace::{LoadError, load_namespaces},
};
use sungma_memory::{MemoryDictionary, MemoryStore};
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
        let namespaces = MemoryStore::default();
        let loaded = load_namespaces(&[document], &dictionary, &namespaces);
        assert!(
            matches!(loaded, Err(LoadError::Intern(InternError::Exhausted(p))) if p == pool),
            "{loaded:?}"
        );
    }
}

/// A fact document looks names up and never interns them, so a refused
/// one leaves the dictionary as it was.
#[tokio::test]
async fn a_refused_fact_document_interns_no_name() {
    let dictionary = MemoryDictionary::default();
    let json = r#"[{ "namespace": "file", "resource": "a", "relation": "owner",
        "subject": { "identity": "alice" } }]"#;
    let facts = MemoryStore::default();
    let loaded = load_facts(json, &dictionary, &facts);
    assert!(
        matches!(&loaded, Err(FactError::UnknownRelation(name)) if name.as_str() == "owner"),
        "{loaded:?}"
    );
    for (pool, name) in [(Pool::Namespaces, "file"), (Pool::Relations, "owner")] {
        assert_eq!(dictionary.lookup(pool, name).await.unwrap(), None);
    }
    assert_eq!(facts.head().await.unwrap(), Revision(0));
}

#[tokio::test]
async fn a_refused_namespace_document_interns_no_name() {
    let dictionary = MemoryDictionary::default();
    let documents = [
        r#"{ "file": { "owner": "this" } }"#,
        r#"{ "folder": { "viewer": "editor" } }"#,
    ];
    let loaded = load_namespaces(&documents, &dictionary, &MemoryStore::default());
    assert!(matches!(loaded, Err(LoadError::Refused(_))), "{loaded:?}");
    for (pool, name) in [(Pool::Namespaces, "file"), (Pool::Relations, "owner")] {
        assert_eq!(dictionary.lookup(pool, name).await.unwrap(), None);
    }
}
