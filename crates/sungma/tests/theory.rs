//! Theories are checked whole when declared.

mod common;

use std::time::{Duration, Instant};

use sungma::{
    id::RelationId,
    memory::{MemoryDictionary, MemoryTheoryStore},
    rewrite::Rewrite::{self, Computed, Exclusion, This, Union},
    theory::{MAX_CYCLE_NAMES, MAX_REWRITE_DEPTH, Problem, Theory, TheoryError},
};
use sungma_lang::theory::{ErrorKind, Refusal, load_theories, parse};

fn load(json: &str) -> Result<(), Refusal> {
    let dictionary = MemoryDictionary::default();
    let mut theories = MemoryTheoryStore::default();
    load_theories(&[json], &dictionary, &mut theories)
}

/// The one theory error a document is refused with.
fn refused(json: &str) -> TheoryError {
    match parse(json) {
        Err(mut errors) if errors.len() == 1 => match errors.remove(0).kind {
            ErrorKind::Theory(error) => error,
            other => panic!("expected a theory error, got {other:?}"),
        },
        other => panic!("expected one error, got {other:?}"),
    }
}

fn names(cycle: &[&str]) -> TheoryError {
    TheoryError::RewriteCycle {
        path: cycle.iter().map(|name| (*name).to_owned()).collect(),
        omitted: 0,
    }
}

#[test]
fn a_cycle_through_a_fact_is_left_to_evaluation() {
    let json = r#"{ "folder": {
        "parent": "this",
        "viewer": "this | (parent, viewer)"
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
fn a_theory_document_is_refused_as_a_whole() {
    let result = load(r#"{ "file": { "owner": "this", "viewer": "editor" } }"#);
    let Err(error) = result else {
        panic!("expected a refused document, got {result:?}");
    };
    assert_eq!(
        error.to_string(),
        "relation 'viewer': relation 'viewer' references 'editor', which the theory doesn't declare"
    );
}

#[test]
fn a_computed_subjectset_must_be_declared() {
    let error = refused(r#"{ "file": { "viewer": "owner" } }"#);
    assert_eq!(
        error.to_string(),
        "relation 'viewer' references 'owner', which the theory doesn't declare"
    );
}

#[test]
fn a_factset_must_be_declared() {
    let json = r#"{ "file": {
        "viewer": "(parent, viewer)"
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
        "viewer": "(parent, anything)"
    } }"#;
    assert!(load(json).is_ok());
}

#[test]
fn a_cycle_of_computed_subjectsets_is_refused() {
    let json = r#"{ "file": {
        "editor": "viewer",
        "viewer": "this | editor"
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
    let json = r#"{ "file": { "viewer": "this ! viewer" } }"#;
    assert_eq!(refused(json), names(&["viewer", "viewer"]));
}

#[test]
fn a_cycle_is_reported_from_where_it_closes() {
    let json = r#"{ "file": {
        "a": "b",
        "b": "c",
        "c": "b"
    } }"#;
    assert_eq!(refused(json), names(&["b", "c", "b"]));
}

#[test]
fn relations_sharing_a_target_are_no_cycle() {
    let json = r#"{ "file": {
        "a": "c",
        "b": "c | a",
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
    let dictionary = MemoryDictionary::default();
    let mut theories = MemoryTheoryStore::default();
    common::declare(&mut theories, &dictionary, "file", "deep", rewrite)
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
    let dictionary = MemoryDictionary::default();
    let mut theories = MemoryTheoryStore::default();
    let mut declare =
        |relation, rewrite| common::declare(&mut theories, &dictionary, "file", relation, rewrite);
    declare("owner", This).unwrap();
    declare("editor", Computed("owner")).unwrap();
    // Relations have their own pool, so owner is 0 and editor is 1. The
    // kept editor is checked before the redeclared owner, which now closes
    // a cycle through it.
    let error = declare("owner", Computed("editor")).unwrap_err();
    assert_eq!(error, names(&["1", "0", "1"]));
}

#[test]
fn a_theory_built_with_a_relation_twice_is_refused() {
    let result = Theory::new(vec![(RelationId(0), This), (RelationId(0), This)]);
    assert_eq!(
        result.err(),
        Some(vec![Problem {
            at: 1,
            error: TheoryError::DuplicateRelation("0".to_owned()),
        }])
    );
}

/// Whether work since `start` stayed under a bound that only quadratic
/// work, or worse, would pass. Only a release build is timed: a debug build
/// on a slow runner can be ten times slower.
fn quick(start: Instant) -> bool {
    cfg!(debug_assertions) || start.elapsed() < Duration::from_secs(5)
}

/// A theory built in code bypasses the document limits, so its checks must
/// hold up at any size: a chain far longer than a document allows.
#[test]
fn a_long_chain_built_in_code_is_checked_without_a_crash() {
    let count = 100_000;
    let chain: Vec<_> = (0..count)
        .map(|i| {
            let rewrite = if i + 1 == count {
                This
            } else {
                Computed(i + 1)
            };
            (i, rewrite)
        })
        .collect();
    let start = Instant::now();
    assert!(Theory::new(chain).is_ok());
    assert!(quick(start));
}

#[test]
fn a_large_cycle_built_in_code_is_one_problem() {
    let count = 100_000;
    let ring: Vec<_> = (0..count).map(|i| (i, Computed((i + 1) % count))).collect();
    let start = Instant::now();
    let problems = Theory::new(ring).unwrap_err();
    assert!(quick(start));
    let [
        Problem {
            at: 0,
            error: TheoryError::RewriteCycle { path, omitted },
        },
    ] = problems.as_slice()
    else {
        panic!("expected one cycle, got {} problems", problems.len());
    };
    assert_eq!(path.len(), MAX_CYCLE_NAMES + 1);
    assert_eq!(*omitted, count - MAX_CYCLE_NAMES);
    assert_eq!(path.first(), path.last());
}

#[test]
fn every_problem_is_returned_in_declaration_order() {
    let problems = Theory::new(vec![
        (0, Computed(9)),
        (1, Union(vec![])),
        (2, Computed(3)),
        (3, Computed(2)),
        (0, This),
    ])
    .unwrap_err();
    let found: Vec<_> = problems.iter().map(|problem| problem.at).collect();
    assert_eq!(found, [0, 1, 2, 4]);
}

#[test]
fn a_cycle_error_names_a_bounded_path() {
    let error = TheoryError::RewriteCycle {
        path: vec!["a".to_owned(), "b".to_owned(), "a".to_owned()],
        omitted: 3,
    };
    assert_eq!(
        error.to_string(),
        "rewrite cycle: a -> b -> ... 3 more -> a"
    );
}

#[test]
fn a_theory_keeps_its_declaration_order() {
    let theory = Theory::new(vec![(2, This), (0, Computed(2)), (1, This)]).unwrap();
    let order: Vec<_> = theory.relations().map(|(relation, _)| *relation).collect();
    assert_eq!(order, [2, 0, 1]);
}

#[test]
fn a_cycle_path_stays_within_its_relations() {
    // a and b reach each other; b also leads out to c, which leads nowhere back.
    let problems = Theory::new(vec![
        (0, Computed(1)),
        (1, Union(vec![Computed(2), Computed(0)])),
        (2, This),
    ])
    .unwrap_err();
    let [Problem { at: 0, error }] = problems.as_slice() else {
        panic!("expected one cycle, got {problems:?}");
    };
    assert_eq!(*error, names(&["0", "1", "0"]));
}
