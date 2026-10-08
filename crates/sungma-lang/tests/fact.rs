//! Fact documents: every part under its own key, and a subject read by
//! its keys.

use sungma::{
    memory::{MemoryDictionary, MemoryFactStore},
    resolve,
    store::{FactStore, Pool},
};
use sungma_lang::fact;

/// The message a fact JSON document is refused with.
fn refused(json: &str) -> String {
    let dictionary = MemoryDictionary::default();
    let facts = MemoryFactStore::default();
    match fact::load_facts(json, &dictionary, &facts) {
        Err(error) => error.to_string(),
        Ok(()) => panic!("accepted: {json}"),
    }
}

/// A one-fact document whose subject is `subject`.
fn with_subject(subject: &str) -> String {
    format!(
        r#"[{{ "theory": "file", "resource": "a", "relation": "owner", "subject": {subject} }}]"#
    )
}

#[tokio::test]
async fn a_subject_is_read_by_its_keys() {
    let dictionary = MemoryDictionary::default();
    let facts = MemoryFactStore::default();
    let json = r#"[
        { "theory": "file", "resource": "a", "relation": "owner",
          "subject": { "identity": "anne@example.com" } },
        { "theory": "file", "resource": "a", "relation": "parent",
          "subject": { "theory": "folder", "resource": "root" } },
        { "theory": "file", "resource": "a", "relation": "viewer",
          "subject": { "theory": "group", "resource": "eng", "relation": "member" } }
    ]"#;
    fact::load_facts(json, &dictionary, &facts).unwrap();
    assert_eq!(facts.head().await.unwrap().0, 3);
    let anne = resolve::identity(&dictionary, "anne@example.com").await;
    assert!(anne.unwrap().is_some());
    let folder = resolve::resource(&dictionary, &"folder:root".parse().unwrap()).await;
    assert!(folder.unwrap().is_some());
}

#[test]
fn a_subject_of_no_known_shape_is_refused() {
    for subject in [
        r#"{}"#,
        r#"{ "identity": "alice", "theory": "folder", "resource": "root" }"#,
        r#"{ "theory": "folder" }"#,
        r#"{ "resource": "root" }"#,
        r#"{ "theory": "group", "relation": "member" }"#,
        r#"{ "identity": "alice", "relation": "member" }"#,
    ] {
        let error = refused(&with_subject(subject));
        assert!(
            error.contains("a subject is an identity"),
            "{subject}: {error}"
        );
    }
}

#[test]
fn a_fact_needs_every_part() {
    for (json, missing) in [
        (
            r#"[{ "resource": "a", "relation": "owner", "subject": { "identity": "x" } }]"#,
            "theory",
        ),
        (
            r#"[{ "theory": "file", "relation": "owner", "subject": { "identity": "x" } }]"#,
            "resource",
        ),
        (
            r#"[{ "theory": "file", "resource": "a", "subject": { "identity": "x" } }]"#,
            "relation",
        ),
        (
            r#"[{ "theory": "file", "resource": "a", "relation": "owner" }]"#,
            "subject",
        ),
    ] {
        let error = refused(json);
        assert!(
            error.contains(&format!("missing field `{missing}`")),
            "{error}"
        );
    }
}

#[test]
fn an_unknown_key_is_refused() {
    let error = refused(
        r#"[{ "theory": "file", "resource": "a", "relation": "owner", "subject": { "identity": "x" }, "note": "x" }]"#,
    );
    assert!(error.contains("unknown field `note`"), "{error}");
    let error = refused(&with_subject(r#"{ "identity": "x", "subject": "y" }"#));
    assert!(error.contains("unknown field `subject`"), "{error}");
}

#[test]
fn a_key_written_twice_is_refused() {
    let error = refused(
        r#"[{ "theory": "file", "theory": "file", "resource": "a", "relation": "owner", "subject": { "identity": "x" } }]"#,
    );
    assert!(error.contains("duplicate field `theory`"), "{error}");
    let error = refused(&with_subject(r#"{ "identity": "x", "identity": "y" }"#));
    assert!(error.contains("duplicate field `identity`"), "{error}");
    let error = refused(
        r#"[{ "theory": "file", "theory": "fi le", "resource": "a", "relation": "owner", "subject": { "identity": "x" } }]"#,
    );
    assert!(error.contains("duplicate field `theory`"), "{error}");
}

#[test]
fn a_name_outside_the_grammar_is_refused() {
    let error = refused(
        r#"[{ "theory": "fi le", "resource": "a", "relation": "owner", "subject": { "identity": "x" } }]"#,
    );
    assert!(error.contains("is not a valid name"), "{error}");
    let error = refused(&with_subject(
        r#"{ "theory": "group", "resource": "eng", "relation": "this" }"#,
    ));
    assert!(error.contains("is reserved"), "{error}");
}

#[test]
fn whitespace_in_a_resource_or_identity_is_refused() {
    for json in [
        r#"[{ "theory": "file", "resource": "a b", "relation": "owner", "subject": { "identity": "x" } }]"#.to_owned(),
        with_subject(r#"{ "identity": "al ice" }"#),
        with_subject(r#"{ "theory": "folder", "resource": "r oot" }"#),
        with_subject(r#"{ "theory": "group", "resource": "e ng", "relation": "member" }"#),
    ] {
        let error = refused(&json);
        assert!(error.contains("holds whitespace"), "{error}");
        assert!(error.contains("line 1 column"), "{error}");
    }
}

#[test]
fn an_error_carries_the_line_and_column_of_the_bad_value() {
    let json = "[{ \"theory\": \"file\",
  \"resource\": \"a b\",
  \"relation\": \"owner\",
  \"subject\": { \"identity\": \"x\" } }]";
    let error = refused(json);
    assert!(error.contains("line 2 column"), "{error}");
}

#[test]
fn an_empty_resource_or_identity_is_refused() {
    let error = refused(&with_subject(r#"{ "identity": "" }"#));
    assert!(error.contains("an identity is empty"), "{error}");
    let error = refused(&with_subject(r#"{ "theory": "folder", "resource": "" }"#));
    assert!(error.contains("a resource is empty"), "{error}");
}

#[tokio::test]
async fn a_refused_document_declares_no_facts() {
    let dictionary = MemoryDictionary::default();
    let facts = MemoryFactStore::default();
    let json = r#"[
        { "theory": "file", "resource": "a", "relation": "owner",
          "subject": { "identity": "alice" } },
        { "theory": "file", "resource": "a", "relation": "owner",
          "subject": { "identity": "bob", "theory": "folder", "resource": "root" } }
    ]"#;
    assert!(fact::load_facts(json, &dictionary, &facts).is_err());
    assert_eq!(facts.head().await.unwrap().0, 0);
    let alice = resolve::identity(&dictionary, "alice").await.unwrap();
    assert_eq!(alice, None);
}

#[test]
fn a_fact_and_a_subject_must_be_objects() {
    let error = refused(r#"["file:a#owner@alice"]"#);
    assert!(error.contains("expected a fact"), "{error}");
    let error = refused(&with_subject(r#""alice""#));
    assert!(error.contains("expected a subject"), "{error}");
}

#[tokio::test]
async fn resource_ids_are_pooled_per_theory() {
    let dictionary = MemoryDictionary::default();
    let facts = MemoryFactStore::default();
    let json = r#"[{ "theory": "file", "resource": "a", "relation": "owner",
        "subject": { "identity": "alice" } }]"#;
    fact::load_facts(json, &dictionary, &facts).unwrap();
    let file = resolve::resource(&dictionary, &"file:a".parse().unwrap()).await;
    assert!(file.unwrap().is_some());
    dictionary.intern(Pool::Theories, "folder");
    let folder = resolve::resource(&dictionary, &"folder:a".parse().unwrap()).await;
    assert_eq!(folder.unwrap(), None);
}
