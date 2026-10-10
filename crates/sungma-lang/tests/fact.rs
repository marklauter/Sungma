//! Fact documents: every part under its own key, and a subject read by
//! its keys.

use std::time::{Duration, Instant};

use proptest::prelude::*;
use serde_json::{Value, json};
use sungma::{
    graph::Subject,
    name::{MAX_IDENTITY_BYTES, MAX_NAME_BYTES, MAX_RESOURCE_BYTES, ResourceName, SubjectsetName},
    store::{FactStore, Pool},
};
use sungma_check::resolve;
use sungma_lang::fact;
use sungma_memory::{MemoryDictionary, MemoryFactStore};

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
        r#"{ "relation": "member" }"#,
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
    for (key, value) in [
        ("resource", r#""a""#),
        ("relation", r#""owner""#),
        ("subject", r#"{ "identity": "x" }"#),
    ] {
        // The repeated key comes before every other, so no other key has
        // been read when it repeats.
        let error = refused(&format!(
            r#"[{{ "{key}": {value}, "{key}": {value}, "theory": "file", "resource": "a", "relation": "owner", "subject": {{ "identity": "x" }} }}]"#
        ));
        assert!(
            error.contains(&format!("duplicate field `{key}`")),
            "{error}"
        );
    }
    for key in ["theory", "resource", "relation"] {
        let error = refused(&with_subject(&format!(
            r#"{{ "{key}": "x", "{key}": "x", "theory": "g", "resource": "e", "relation": "m" }}"#
        )));
        assert!(
            error.contains(&format!("duplicate field `{key}`")),
            "{error}"
        );
    }
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
    let file = resolve::resource(&dictionary, &"file:a".parse().unwrap()).await;
    assert_eq!(file.unwrap(), None);
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

/// The subject of a fact, as a test writes it.
#[derive(Clone, Debug)]
enum Who {
    Identity(String),
    Resource(String, String),
    Subjectset(String, String, String),
}

impl Who {
    fn json(&self) -> Value {
        match self {
            Self::Identity(identity) => json!({ "identity": identity }),
            Self::Resource(theory, resource) => json!({ "theory": theory, "resource": resource }),
            Self::Subjectset(theory, resource, relation) => {
                json!({ "theory": theory, "resource": resource, "relation": relation })
            }
        }
    }
}

fn fact_json(theory: &str, resource: &str, relation: &str, subject: &Who) -> Value {
    json!({ "theory": theory, "resource": resource, "relation": relation, "subject": subject.json() })
}

fn loaded(json: &str) -> (MemoryDictionary, MemoryFactStore) {
    let dictionary = MemoryDictionary::default();
    let facts = MemoryFactStore::default();
    if let Err(error) = fact::load_facts(json, &dictionary, &facts) {
        panic!("refused: {error}");
    }
    (dictionary, facts)
}

fn set_name(theory: &str, resource: &str, relation: &str) -> SubjectsetName {
    let resource = ResourceName::new(theory.parse().unwrap(), resource).unwrap();
    SubjectsetName::new(resource, relation.parse().unwrap())
}

/// Whether the store holds the fact, found under the names it was written
/// with.
async fn holds(
    (dictionary, facts): &(MemoryDictionary, MemoryFactStore),
    (theory, resource, relation): (&str, &str, &str),
    subject: &Who,
) -> bool {
    let set = set_name(theory, resource, relation);
    let Some(set) = resolve::subjectset(dictionary, &set).await.unwrap() else {
        return false;
    };
    let subject = match subject {
        Who::Identity(identity) => resolve::identity(dictionary, identity)
            .await
            .unwrap()
            .map(Subject::Identity),
        Who::Resource(theory, resource) => {
            let name = ResourceName::new(theory.parse().unwrap(), resource).unwrap();
            resolve::resource(dictionary, &name)
                .await
                .unwrap()
                .map(Subject::ResourceMember)
        }
        Who::Subjectset(theory, resource, relation) => {
            resolve::subjectset(dictionary, &set_name(theory, resource, relation))
                .await
                .unwrap()
                .map(Subject::Subjectset)
        }
    };
    let Some(subject) = subject else {
        return false;
    };
    let head = facts.head().await.unwrap();
    facts.contains(set, subject, head).await.unwrap()
}

fn identity(text: &str) -> Who {
    Who::Identity(text.to_owned())
}

#[tokio::test]
async fn an_empty_document_holds_no_facts() {
    let (_, facts) = loaded("[]");
    assert_eq!(facts.head().await.unwrap().0, 0);
}

#[test]
fn a_document_must_be_an_array() {
    for json in ["{}", r#""facts""#, "null"] {
        let error = refused(json);
        assert!(error.contains("expected a sequence"), "{json}: {error}");
    }
}

#[test]
fn a_name_must_be_a_string() {
    for json in [
        r#"[{ "theory": 5, "resource": "a", "relation": "owner", "subject": { "identity": "x" } }]"#
            .to_owned(),
        r#"[{ "theory": "file", "resource": null, "relation": "owner", "subject": { "identity": "x" } }]"#
            .to_owned(),
        with_subject(r#"{ "identity": ["x"] }"#),
        with_subject(r#"{ "theory": "folder", "resource": {} }"#),
    ] {
        let error = refused(&json);
        assert!(error.contains("invalid type"), "{json}: {error}");
    }
}

#[test]
fn this_is_reserved_in_any_casing() {
    for relation in ["this", "This", "THIS"] {
        let fact = fact_json("file", "a", relation, &identity("x"));
        let error = refused(&json!([fact]).to_string());
        assert!(error.contains("is reserved"), "{relation}: {error}");
        let subject = Who::Subjectset("group".into(), "eng".into(), relation.into());
        let fact = fact_json("file", "a", "viewer", &subject);
        let error = refused(&json!([fact]).to_string());
        assert!(error.contains("is reserved"), "{relation}: {error}");
    }
}

#[tokio::test]
async fn opaque_text_is_kept_as_written() {
    let texts = [
        "a:b#c@d",
        "this/is/a/path/resource-id",
        "file://myfile.md",
        r"C:\docs\a.md",
        "a+b=c>d",
        "anne@example.com",
        "é🦀",
    ];
    let mut documents = Vec::new();
    for text in texts {
        documents.push(fact_json("file", text, "owner", &identity(text)));
        documents.push(fact_json(
            "file",
            "a",
            "parent",
            &Who::Resource("folder".into(), text.into()),
        ));
    }
    let world = loaded(&json!(documents).to_string());
    for text in texts {
        assert!(
            holds(&world, ("file", text, "owner"), &identity(text)).await,
            "{text}"
        );
        let folder = Who::Resource("folder".into(), text.into());
        assert!(
            holds(&world, ("file", "a", "parent"), &folder).await,
            "{text}"
        );
    }
}

#[test]
fn whitespace_of_every_kind_is_refused() {
    for space in [" ", "\t", "\n", "\r", "\u{a0}", "\u{2003}", "\u{3000}"] {
        let text = format!("a{space}b");
        let fact = fact_json("file", &text, "owner", &identity("x"));
        let error = refused(&json!([fact]).to_string());
        assert!(error.contains("holds whitespace"), "{text:?}: {error}");
        let fact = fact_json("file", "a", "owner", &identity(&text));
        let error = refused(&json!([fact]).to_string());
        assert!(error.contains("holds whitespace"), "{text:?}: {error}");
    }
}

#[tokio::test]
async fn case_is_kept() {
    let documents = json!([
        fact_json("file", "a", "owner", &identity("Anne")),
        fact_json("file", "a", "owner", &identity("anne")),
    ]);
    let world = loaded(&documents.to_string());
    assert_eq!(world.1.head().await.unwrap().0, 2);
    let upper = resolve::identity(&world.0, "Anne").await.unwrap();
    let lower = resolve::identity(&world.0, "anne").await.unwrap();
    assert!(upper.is_some() && lower.is_some() && upper != lower);
}

#[test]
fn a_resource_is_at_most_max_resource_bytes() {
    let resource = |len: usize| "x".repeat(len - "file:".len());
    let fact = fact_json(
        "file",
        &resource(MAX_RESOURCE_BYTES),
        "owner",
        &identity("x"),
    );
    loaded(&json!([fact]).to_string());
    let too_long = format!("longer than {MAX_RESOURCE_BYTES} bytes");
    let fact = fact_json(
        "file",
        &resource(MAX_RESOURCE_BYTES + 1),
        "owner",
        &identity("x"),
    );
    let error = refused(&json!([fact]).to_string());
    assert!(error.contains(&too_long), "{error}");
    let subject = Who::Resource("file".into(), resource(MAX_RESOURCE_BYTES + 1));
    let error = refused(&json!([fact_json("file", "a", "parent", &subject)]).to_string());
    assert!(error.contains(&too_long), "{error}");
}

#[test]
fn an_identity_is_at_most_max_identity_bytes() {
    let fact = fact_json(
        "file",
        "a",
        "owner",
        &identity(&"x".repeat(MAX_IDENTITY_BYTES)),
    );
    loaded(&json!([fact]).to_string());
    let fact = fact_json(
        "file",
        "a",
        "owner",
        &identity(&"x".repeat(MAX_IDENTITY_BYTES + 1)),
    );
    let error = refused(&json!([fact]).to_string());
    assert!(
        error.contains(&format!("longer than {MAX_IDENTITY_BYTES} bytes")),
        "{error}"
    );
}

#[test]
fn a_name_is_at_most_max_name_bytes() {
    let longest = format!("n{}", "x".repeat(MAX_NAME_BYTES - 1));
    let longer = format!("{longest}x");
    loaded(&json!([fact_json(&longest, "a", &longest, &identity("x"))]).to_string());
    for fact in [
        fact_json(&longer, "a", "owner", &identity("x")),
        fact_json("file", "a", &longer, &identity("x")),
    ] {
        let error = refused(&json!([fact]).to_string());
        assert!(
            error.contains(&format!("longer than {MAX_NAME_BYTES} bytes")),
            "{error}"
        );
    }
}

#[test]
fn an_error_quotes_a_bounded_and_escaped_cut_of_the_text() {
    let long = format!("{} x", "x".repeat(100));
    let fact = fact_json("file", "a", "owner", &identity(&long));
    let error = refused(&json!([fact]).to_string());
    assert!(
        error.contains(&format!("'{}...'", "x".repeat(64))),
        "{error}"
    );
    let hidden = "a\u{202e}\u{200b} b";
    let fact = fact_json("file", hidden, "owner", &identity("x"));
    let error = refused(&json!([fact]).to_string());
    assert!(error.contains(r"a\u{202e}\u{200b} b"), "{error}");
}

#[test]
fn a_deeply_nested_value_is_refused_without_a_crash() {
    let deep =
        |open: &str, close: &str| format!("{}{}", open.repeat(100_000), close.repeat(100_000));
    for json in [
        with_subject(&deep("[", "]")),
        with_subject(&format!(r#"{{ "identity": {} }}"#, deep("[", "]"))),
        with_subject(&deep(r#"{ "identity": "#, "}")),
        format!(r#"[{{ "theory": {} }}]"#, deep(r#"{"a":"#, "}")),
        deep("[", "]"),
    ] {
        refused(&json);
    }
}

#[tokio::test]
async fn a_large_document_loads_within_a_time_bound() {
    let documents: Vec<Value> = (0..100_000)
        .map(|i| {
            fact_json(
                "file",
                &format!("f{i}"),
                "owner",
                &identity(&format!("p{}", i % 97)),
            )
        })
        .collect();
    let json = json!(documents).to_string();
    let started = Instant::now();
    let (_, facts) = loaded(&json);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(facts.head().await.unwrap().0, 100_000);
}

#[tokio::test]
async fn a_json_escape_is_decoded_before_the_rules_apply() {
    let error = refused(&with_subject(r#"{ "identity": "a\u0020b" }"#));
    assert!(error.contains("holds whitespace"), "{error}");
    let world = loaded(&with_subject(r#"{ "identity": "C:\\docs" }"#));
    assert!(holds(&world, ("file", "a", "owner"), &identity(r"C:\docs")).await);
    let world = loaded(&with_subject(r#"{ "identity": "a\u0000b" }"#));
    assert!(holds(&world, ("file", "a", "owner"), &identity("a\0b")).await);
}

#[test]
fn a_raw_nul_and_a_byte_order_mark_are_refused() {
    refused(&with_subject("{ \"identity\": \"a\0b\" }"));
    refused(&format!(
        "\u{feff}{}",
        with_subject(r#"{ "identity": "x" }"#)
    ));
}

fn name() -> impl Strategy<Value = String> {
    "[a-z_][a-z0-9_]{0,8}".prop_filter("not this", |name| !name.eq_ignore_ascii_case("this"))
}

fn who() -> impl Strategy<Value = Who> {
    prop_oneof![
        r"\S{1,20}".prop_map(Who::Identity),
        (name(), r"\S{1,20}").prop_map(|(theory, resource)| Who::Resource(theory, resource)),
        (name(), r"\S{1,20}", name())
            .prop_map(|(theory, resource, relation)| Who::Subjectset(theory, resource, relation)),
    ]
}

proptest! {
    /// Generated documents load, and each fact is found under its names.
    #[test]
    fn a_generated_document_loads_and_each_fact_is_found(
        facts in proptest::collection::vec((name(), r"\S{1,20}", name(), who()), 1..10),
    ) {
        let documents: Vec<Value> = facts
            .iter()
            .map(|(theory, resource, relation, subject)| fact_json(theory, resource, relation, subject))
            .collect();
        let world = loaded(&json!(documents).to_string());
        let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
        for (theory, resource, relation, subject) in &facts {
            let found = runtime.block_on(holds(&world, (theory, resource, relation), subject));
            prop_assert!(found, "{theory}:{resource}#{relation} {subject:?}");
        }
    }

    /// Arbitrary text never panics the parser, and a refused document
    /// declares no facts.
    #[test]
    fn arbitrary_text_never_panics(
        text in prop_oneof![
            any::<String>(),
            r#"[\[\]{}":, a-z0-9#@\\]{0,64}"#,
            r#"\[\{ "(theory|resource|relation|subject|identity)": ("[a-z :]{0,4}"|\{\}|[0-9])[,}\]]{0,3}"#,
        ],
    ) {
        let dictionary = MemoryDictionary::default();
        let facts = MemoryFactStore::default();
        if fact::load_facts(&text, &dictionary, &facts).is_err() {
            let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
            prop_assert_eq!(runtime.block_on(facts.head()).unwrap().0, 0);
        }
    }
}
