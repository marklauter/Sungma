//! Theories are checked whole when declared.

use sungma_core::{
    fixture::{self, FixtureError},
    memory::{MemoryDictionary, MemoryTheoryStore},
    rewrite::Rewrite::{self, Computed, Exclusion, This, Union},
    theory::{MAX_REWRITE_DEPTH, TheoryError},
};

fn load(json: &str) -> Result<(), FixtureError> {
    let mut dictionary = MemoryDictionary::default();
    let mut theories = MemoryTheoryStore::default();
    fixture::load_theories(json, &mut dictionary, &mut theories)
}

fn refused(json: &str) -> TheoryError {
    match load(json) {
        Err(FixtureError::Theory(error)) => error,
        other => panic!("expected a theory error, got {other:?}"),
    }
}

fn names(cycle: &[&str]) -> TheoryError {
    TheoryError::RewriteCycle(cycle.iter().map(|name| (*name).to_owned()).collect())
}

#[test]
fn a_cycle_through_a_fact_is_left_to_evaluation() {
    let json = r#"{ "folder": {
        "parent": "this",
        "viewer": { "union": ["this", { "fact_to": { "factset": "parent", "computed": "viewer" } }] }
    } }"#;
    assert!(load(json).is_ok());
}

#[test]
fn a_relation_declared_twice_is_refused() {
    let error = refused(r#"{ "file": { "owner": "this", "owner": "this" } }"#);
    assert_eq!(error, TheoryError::DuplicateRelation("owner".to_owned()));
    assert_eq!(
        error.to_string(),
        "relation 'owner' is declared more than once"
    );
}

#[test]
fn a_theory_declared_twice_is_refused() {
    let result = load(r#"{ "file": { "owner": "this" }, "file": { "owner": "this" } }"#);
    let Err(error @ FixtureError::DuplicateTheory(_)) = result else {
        panic!("expected a duplicate theory, got {result:?}");
    };
    assert_eq!(
        error.to_string(),
        "theory 'file' is declared more than once"
    );
}

#[test]
fn a_computed_subjectset_must_be_declared() {
    let error = refused(r#"{ "file": { "viewer": { "computed": "owner" } } }"#);
    assert_eq!(
        error.to_string(),
        "relation 'viewer' references 'owner', which the theory doesn't declare"
    );
}

#[test]
fn a_factset_must_be_declared() {
    let json = r#"{ "file": {
        "viewer": { "fact_to": { "factset": "parent", "computed": "viewer" } }
    } }"#;
    assert_eq!(
        refused(json),
        TheoryError::DanglingReference {
            relation: "viewer".to_owned(),
            target: "parent".to_owned(),
        }
    );
}

#[test]
fn the_relation_a_fact_to_subjectset_computes_is_not_checked() {
    let json = r#"{ "file": {
        "parent": "this",
        "viewer": { "fact_to": { "factset": "parent", "computed": "anything" } }
    } }"#;
    assert!(load(json).is_ok());
}

#[test]
fn a_cycle_of_computed_subjectsets_is_refused() {
    let json = r#"{ "file": {
        "editor": { "computed": "viewer" },
        "viewer": { "union": ["this", { "computed": "editor" }] }
    } }"#;
    let error = refused(json);
    assert_eq!(error, names(&["editor", "viewer", "editor"]));
    assert_eq!(
        error.to_string(),
        "rewrite cycle: editor -> viewer -> editor"
    );
}

#[test]
fn a_relation_excluding_itself_is_refused() {
    let json = r#"{ "file": { "viewer": { "exclusion": ["this", { "computed": "viewer" }] } } }"#;
    assert_eq!(refused(json), names(&["viewer", "viewer"]));
}

#[test]
fn a_cycle_is_reported_from_where_it_closes() {
    let json = r#"{ "file": {
        "a": { "computed": "b" },
        "b": { "computed": "c" },
        "c": { "computed": "b" }
    } }"#;
    assert_eq!(refused(json), names(&["b", "c", "b"]));
}

#[test]
fn relations_sharing_a_target_are_no_cycle() {
    let json = r#"{ "file": {
        "a": { "computed": "c" },
        "b": { "union": [{ "computed": "c" }, { "computed": "a" }] },
        "c": "this"
    } }"#;
    assert!(load(json).is_ok());
}

/// A chain of single-operand unions, `levels` deep counting the leaf.
fn nested(levels: usize) -> Rewrite<&'static str> {
    nested_in(levels, |rewrite| Union(vec![rewrite]))
}

/// A chain of `wrap`s around `this`, `levels` deep counting the leaf.
fn nested_in(
    levels: usize,
    wrap: fn(Rewrite<&'static str>) -> Rewrite<&'static str>,
) -> Rewrite<&'static str> {
    (1..levels).fold(This, |rewrite, _| wrap(rewrite))
}

fn declare(rewrite: Rewrite<&str>) -> Result<(), TheoryError> {
    let mut dictionary = MemoryDictionary::default();
    let mut theories = MemoryTheoryStore::default();
    fixture::declare(&mut theories, &mut dictionary, "file", "deep", rewrite)
}

#[test]
fn a_rewrite_tree_may_be_max_depth_deep() {
    assert_eq!(declare(nested(MAX_REWRITE_DEPTH)), Ok(()));
}

#[test]
fn a_deeper_rewrite_tree_is_refused() {
    let error = declare(nested(MAX_REWRITE_DEPTH + 1)).unwrap_err();
    assert_eq!(error, TheoryError::TooDeep);
    assert_eq!(
        error.to_string(),
        "a rewrite tree deeper than 100 levels is refused"
    );
}

#[test]
fn a_deeper_exclusion_base_is_refused() {
    let base = nested_in(MAX_REWRITE_DEPTH + 1, |rewrite| {
        Exclusion(Box::new(rewrite), Box::new(This))
    });
    assert_eq!(declare(base), Err(TheoryError::TooDeep));
}

#[test]
fn a_deeper_excluded_operand_is_refused() {
    let excluded = nested_in(MAX_REWRITE_DEPTH + 1, |rewrite| {
        Exclusion(Box::new(This), Box::new(rewrite))
    });
    assert_eq!(declare(excluded), Err(TheoryError::TooDeep));
}

#[test]
fn redeclaring_a_relation_checks_the_whole_theory_by_id() {
    let mut dictionary = MemoryDictionary::default();
    let mut theories = MemoryTheoryStore::default();
    let mut declare = |relation, rewrite| {
        fixture::declare(&mut theories, &mut dictionary, "file", relation, rewrite)
    };
    declare("owner", This).unwrap();
    declare("editor", Computed("owner")).unwrap();
    // owner is 1 and editor is 2. The kept editor is checked before the
    // redeclared owner, which now closes a cycle through it.
    let error = declare("owner", Computed("editor")).unwrap_err();
    assert_eq!(error, names(&["2", "1", "2"]));
}

#[test]
fn theory_json_must_be_objects() {
    assert!(matches!(load("[]"), Err(FixtureError::Json(_))));
    assert!(matches!(
        load(r#"{ "file": [] }"#),
        Err(FixtureError::Json(_))
    ));
}
