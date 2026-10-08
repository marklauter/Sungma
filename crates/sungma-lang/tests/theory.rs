//! Theory documents: parsed, checked whole, and printed back.

use std::{
    panic::{self, AssertUnwindSafe},
    time::{Duration, Instant},
};

use proptest::{collection::vec, prelude::*};
use sungma::{
    name::{NameError, RelationName, TheoryName},
    rewrite::Rewrite::{self, Computed, Exclusion, FactTo, Intersection, This, Union},
    theory::{MAX_REWRITE_DEPTH, Theory, TheoryError},
};
use sungma_lang::theory::{
    self, DocumentError, ErrorKind, MAX_DOCUMENT_BYTES, MAX_ERRORS, MAX_EXPRESSION_BYTES,
    MAX_NAME_BYTES, MAX_QUOTE_BYTES, MAX_RELATIONS, TheoryDocument,
};

fn parsed(json: &str) -> TheoryDocument {
    theory::parse(json).unwrap_or_else(|errors| panic!("refused: {errors:?}"))
}

fn refused(json: &str) -> Vec<DocumentError> {
    match theory::parse(json) {
        Err(errors) => errors,
        Ok(document) => panic!("accepted: {document:?}"),
    }
}

/// The one error a document is refused with.
fn only(json: &str) -> DocumentError {
    let mut errors = refused(json);
    assert_eq!(errors.len(), 1, "{errors:?}");
    errors.remove(0)
}

/// The rewrite of the one relation `r`.
fn rewrite(expression: &str) -> Rewrite<RelationName> {
    let json = format!(
        r#"{{ "t": {{ "r": "{expression}", "a": "this", "b": "this", "c": "this", "p": "this" }} }}"#
    );
    let document = parsed(&json);
    listed(&document)[0].1.clone()
}

/// The error the expression of relation `r` is refused with, and its column.
fn syntax(expression: &str) -> (Option<usize>, ErrorKind) {
    let json = format!(r#"{{ "t": {{ "r": "{expression}", "a": "this", "b": "this" }} }}"#);
    let error = only(&json);
    assert_eq!(error.relation.as_deref(), Some("r"));
    (error.column, error.kind)
}

fn relation(name: &str) -> RelationName {
    RelationName::new_unchecked(name)
}

/// The theory `t`.
fn t() -> TheoryName {
    TheoryName::new_unchecked("t")
}

fn name(name: &str) -> Rewrite<RelationName> {
    Computed(relation(name))
}

fn fact_to(factset: &str, computed: &str) -> Rewrite<RelationName> {
    FactTo {
        factset: relation(factset),
        computed: relation(computed),
    }
}

/// A theory's relations, named as written.
type Relations = Vec<(&'static str, Rewrite<&'static str>)>;

/// A valid theory of relations written with plain names, each made with
/// `new_unchecked`.
fn checked(relations: &[(&str, Rewrite<&str>)]) -> Theory<RelationName> {
    let relations = relations
        .iter()
        .map(|(name, rewrite)| {
            (
                relation(name),
                rewrite.clone().map(&mut |name| relation(name)),
            )
        })
        .collect();
    Theory::new(relations).expect("a valid theory")
}

fn print(theory: &str, relations: &[(&str, Rewrite<&str>)]) -> String {
    theory::print(&TheoryName::new_unchecked(theory), &checked(relations))
}

/// A document's relations, in document order.
fn listed(document: &TheoryDocument) -> Vec<(RelationName, Rewrite<RelationName>)> {
    document
        .relations
        .relations()
        .map(|(name, rewrite)| (name.clone(), rewrite.clone()))
        .collect()
}

/// How the printer writes `tree` as the rewrite of relation `r`, beside
/// every relation in [`DECLARED`].
fn printed_expression(tree: Rewrite<RelationName>) -> String {
    let mut relations: Vec<_> = DECLARED.iter().map(|name| (relation(name), This)).collect();
    relations.push((relation("r"), tree));
    let theory = Theory::new(relations).expect("a valid theory");
    let text = theory::print(&t(), &theory);
    let line = text
        .lines()
        .find(|line| line.starts_with("    \"r\": "))
        .expect("relation r is printed");
    line.trim_start_matches("    \"r\": \"")
        .trim_end_matches(',')
        .trim_end_matches('"')
        .to_owned()
}

fn exclusion<N>(base: Rewrite<N>, excluded: Rewrite<N>) -> Rewrite<N> {
    Exclusion(Box::new(base), Box::new(excluded))
}

#[test]
fn the_sample_parses() {
    let document = parsed(include_str!(
        "../../sungma/tests/fixtures/theories/file.json"
    ));
    assert_eq!(document.theory.as_str(), "file");
    let viewer = &document
        .relations
        .relations()
        .find(|(r, _)| r.as_str() == "viewer")
        .unwrap()
        .1;
    assert_eq!(
        **viewer,
        exclusion(
            Union(vec![This, name("editor"), fact_to("parent", "viewer")]),
            name("banned")
        )
    );
}

#[test]
fn relations_keep_document_order() {
    let document = parsed(r#"{ "t": { "b": "this", "a": "b" } }"#);
    let names: Vec<_> = document
        .relations
        .relations()
        .map(|(r, _)| r.as_str())
        .collect();
    assert_eq!(names, ["b", "a"]);
}

#[test]
fn exclusion_binds_tightest_then_intersection() {
    assert_eq!(
        rewrite("a | b & c ! p"),
        Union(vec![
            name("a"),
            Intersection(vec![name("b"), exclusion(name("c"), name("p"))])
        ])
    );
}

#[test]
fn exclusion_reads_left_to_right() {
    assert_eq!(
        rewrite("a ! b ! c"),
        exclusion(exclusion(name("a"), name("b")), name("c"))
    );
}

#[test]
fn a_run_of_one_operator_is_one_node() {
    assert_eq!(
        rewrite("a | b | c"),
        Union(vec![name("a"), name("b"), name("c")])
    );
    assert_eq!(
        rewrite("a & b & c"),
        Intersection(vec![name("a"), name("b"), name("c")])
    );
}

#[test]
fn parentheses_are_kept_as_structure() {
    assert_eq!(
        rewrite("(a | b) | c"),
        Union(vec![Union(vec![name("a"), name("b")]), name("c")])
    );
    assert_eq!(rewrite("((a))"), name("a"));
}

#[test]
fn whitespace_only_separates_tokens() {
    assert_eq!(rewrite("(p,a)|this"), rewrite(" ( p , a )  |  this "));
    assert_eq!(rewrite("(p,a)|this"), Union(vec![fact_to("p", "a"), This]));
}

#[test]
fn a_name_is_the_longest_run_of_name_characters() {
    let document = parsed(r#"{ "t": { "thisone": "this", "r": "thisone" } }"#);
    assert_eq!(listed(&document)[1].1, name("thisone"));
}

#[test]
fn a_dotted_theory_name_is_one_name() {
    assert_eq!(
        parsed(r#"{ "drive.file": { "owner": "this" } }"#)
            .theory
            .as_str(),
        "drive.file"
    );
}

#[test]
fn a_theory_without_relations_is_empty() {
    assert!(
        parsed(r#"{ "t": {} }"#)
            .relations
            .relations()
            .next()
            .is_none()
    );
}

#[test]
fn syntax_errors_carry_their_column() {
    let expected = |found: &str, expected: &str| ErrorKind::Syntax {
        found: found.to_owned(),
        expected: match expected {
            "term" => "'this', a relation name or '('",
            "end" => "an operator or the end",
            "close" => "')'",
            _ => "a relation name",
        },
    };
    let cases = [
        ("a |", 4, expected("the end", "term")),
        ("a b", 3, expected("'b'", "end")),
        ("(a | b", 7, expected("the end", "close")),
        ("a | )", 5, expected("')'", "term")),
        ("(this, a)", 2, expected("'this'", "relation")),
        ("(a, this)", 5, expected("'this'", "relation")),
        ("(a, (b))", 5, expected("'('", "relation")),
        ("(a, b c)", 7, expected("'c'", "close")),
        ("a | é", 5, expected("'é'", "term")),
        ("é | #", 1, expected("'é'", "term")),
    ];
    for (expression, column, kind) in cases {
        let (at, found) = syntax(expression);
        assert_eq!(at, Some(column), "{expression}");
        assert_eq!(found.to_string(), kind.to_string(), "{expression}");
    }
}

#[test]
fn this_in_any_other_case_is_reserved() {
    for expression in ["This", "a | THIS", "(tHiS, a)", "(a, This)"] {
        let (_, kind) = syntax(expression);
        assert!(
            matches!(kind, ErrorKind::Name(NameError::Reserved(_))),
            "{expression}: {kind}"
        );
    }
    for relation in ["this", "This"] {
        let error = only(&format!(r#"{{ "t": {{ "{relation}": "this" }} }}"#));
        assert!(
            matches!(error.kind, ErrorKind::Name(NameError::Reserved(_))),
            "{relation}"
        );
    }
}

#[test]
fn names_follow_the_grammar() {
    for json in [
        r#"{ "t": { "1a": "this" } }"#,
        r#"{ "t": { "a-b": "this" } }"#,
        r#"{ "t": { "": "this" } }"#,
        r#"{ "t.": { "a": "this" } }"#,
        r#"{ "t:x": { "a": "this" } }"#,
    ] {
        assert!(
            matches!(only(json).kind, ErrorKind::Name(NameError::Invalid(_))),
            "{json}"
        );
    }
    let long = "n".repeat(65);
    for json in [
        format!(r#"{{ "t": {{ "{long}": "this" }} }}"#),
        format!(r#"{{ "{long}": {{ "a": "this" }} }}"#),
    ] {
        assert!(
            matches!(only(&json).kind, ErrorKind::Name(NameError::TooLong(_))),
            "{json}"
        );
    }
    let longest = "n".repeat(64);
    parsed(&format!(r#"{{ "{longest}": {{ "{longest}": "this" }} }}"#));
}

#[test]
fn a_relation_maps_to_a_non_empty_string() {
    for value in [
        "null",
        r#""""#,
        "1",
        "-1",
        "1.5",
        "true",
        "{}",
        r#"["this"]"#,
    ] {
        let error = only(&format!(r#"{{ "t": {{ "r": {value} }} }}"#));
        assert!(matches!(error.kind, ErrorKind::NotAnExpression), "{value}");
    }
}

#[test]
fn a_document_declares_exactly_one_theory() {
    assert!(matches!(only("{}").kind, ErrorKind::NoTheory));
    let error = only(r#"{ "a": {}, "b": {} }"#);
    assert_eq!(error.to_string(), "a document declares one theory, not 2");
    let error = only(r#"{ "\u0061": {}, "b": {} }"#);
    assert_eq!(error.to_string(), "a document declares one theory, not 2");
    assert!(matches!(
        only(r#"{ "a": {}, "a": {} }"#).kind,
        ErrorKind::ManyTheories(2)
    ));
    for json in ["[]", "null", r#""t""#] {
        assert!(matches!(only(json).kind, ErrorKind::NotAnObject), "{json}");
    }
    for json in [r#"{ "t": null }"#, r#"{ "t": [] }"#] {
        assert!(
            matches!(only(json).kind, ErrorKind::RelationsNotAnObject(_)),
            "{json}"
        );
    }
}

#[test]
fn a_document_that_is_not_json_has_the_library_error() {
    let error = only("{ \"t\": {\n  \"a\": this } }");
    let ErrorKind::Json(json) = &error.kind else {
        panic!("expected a JSON error, got {error}");
    };
    assert_eq!((json.line(), json.column()), (2, 9));
    assert_eq!(error.relation, None);
}

#[test]
fn escapes_are_refused() {
    for json in [
        r#"{ "t": { "\u0061": "this" } }"#,
        r#"{ "t": { "a": "this |\nthis" } }"#,
        r#"{ "\u0074": { "a": "this" } }"#,
    ] {
        assert!(matches!(only(json).kind, ErrorKind::Escape(_)), "{json}");
    }
}

#[test]
fn a_relation_named_twice_is_one_error() {
    let error = only(r#"{ "t": { "a": "this", "a": "this" } }"#);
    assert!(matches!(
        error.kind,
        ErrorKind::Theory(TheoryError::DuplicateRelation(_))
    ));
}

#[test]
fn parentheses_nest_at_most_max_depth() {
    let nested = |depth: usize| format!("{}a{}", "(".repeat(depth), ")".repeat(depth));
    assert_eq!(rewrite(&nested(MAX_REWRITE_DEPTH)), name("a"));
    let (column, kind) = syntax(&nested(MAX_REWRITE_DEPTH + 1));
    assert_eq!(column, Some(MAX_REWRITE_DEPTH + 1));
    assert!(matches!(kind, ErrorKind::TooManyParentheses));
}

#[test]
fn a_rewrite_nests_at_most_max_depth() {
    let chain = |operators: usize| format!("a{}", " ! b".repeat(operators));
    rewrite(&chain(MAX_REWRITE_DEPTH - 1));
    let (column, kind) = syntax(&chain(MAX_REWRITE_DEPTH));
    assert_eq!(column, None);
    assert!(matches!(kind, ErrorKind::Theory(TheoryError::TooDeep)));
    // Unions in groups: each level is a run, one level however wide.
    let unions =
        |levels: usize| (1..levels).fold("a".to_owned(), |inner, _| format!("(a | {inner})"));
    rewrite(&unions(MAX_REWRITE_DEPTH));
    assert!(matches!(
        syntax(&unions(MAX_REWRITE_DEPTH + 1)).1,
        ErrorKind::Theory(TheoryError::TooDeep)
    ));
}

#[test]
fn every_problem_is_reported_in_document_order() {
    let errors = refused(
        r#"{ "t": {
            "a": "b |",
            "b": "missing",
            "c": "a & b",
            "d": "e",
            "e": "d",
            "f": "(b",
            "g": "f ! f"
        } }"#,
    );
    let found: Vec<_> = errors
        .iter()
        .map(|error| (error.relation.as_deref().unwrap(), error.column))
        .collect();
    assert_eq!(
        found,
        [("a", Some(4)), ("b", None), ("d", None), ("f", Some(3))]
    );
    assert!(matches!(
        &errors[2].kind,
        ErrorKind::Theory(TheoryError::RewriteCycle { path, .. }) if path == &["d", "e", "d"]
    ));
}

#[test]
fn a_dangling_target_is_reported_once_per_relation() {
    let error = only(r#"{ "t": { "a": "x | x & (x, b)" } }"#);
    assert_eq!(
        error.to_string(),
        "relation 'a': relation 'a' references 'x', which the theory doesn't declare"
    );
}

#[test]
fn relations_that_reach_each_other_are_one_cycle() {
    let error = only(
        r#"{ "t": {
            "x": "a",
            "a": "b | c",
            "b": "a",
            "c": "a | c"
        } }"#,
    );
    assert_eq!(error.relation.as_deref(), Some("a"));
    assert!(matches!(
        error.kind,
        ErrorKind::Theory(TheoryError::RewriteCycle { path, .. }) if path == ["a", "b", "a"]
    ));
}

#[test]
fn errors_stop_at_the_cap() {
    let relations: Vec<_> = (0..MAX_ERRORS + 10)
        .map(|i| format!(r#""r{i}": "|""#))
        .collect();
    let errors = refused(&format!(r#"{{ "t": {{ {} }} }}"#, relations.join(", ")));
    assert_eq!(errors.len(), MAX_ERRORS + 1);
    assert_eq!(errors[0].relation.as_deref(), Some("r0"));
    assert!(matches!(errors[MAX_ERRORS].kind, ErrorKind::TooManyErrors));
}

#[test]
fn errors_name_their_place() {
    assert_eq!(
        syntax("a |").1.to_string(),
        "expected 'this', a relation name or '(', found the end"
    );
    let error = only(r#"{ "t": { "r": "a |" } }"#);
    assert_eq!(
        error.to_string(),
        "relation 'r', column 4: expected 'this', a relation name or '(', found the end"
    );
    assert_eq!(only("{}").to_string(), "a document declares no theory");
}

#[test]
fn the_printer_writes_the_canonical_form() {
    let relations = vec![
        (
            "viewer",
            exclusion(
                Union(vec![
                    This,
                    Computed("editor"),
                    FactTo {
                        factset: "parent",
                        computed: "viewer",
                    },
                ]),
                Computed("banned"),
            ),
        ),
        ("editor", Union(vec![This, Computed("owner")])),
        ("owner", This),
        ("parent", This),
        ("banned", This),
        ("auditor", Intersection(vec![This, Computed("viewer")])),
    ];
    assert_eq!(
        print("file", &relations),
        include_str!("../../sungma/tests/fixtures/theories/file.json")
    );
    assert_eq!(print("t", &[]), "{\n  \"t\": {}\n}\n");
}

#[test]
fn the_printer_adds_only_the_parentheses_the_grammar_needs() {
    let cases = [
        (
            Union(vec![Union(vec![name("a"), name("b")]), name("c")]),
            "(a | b) | c",
        ),
        (
            Union(vec![
                Intersection(vec![name("a"), name("b")]),
                exclusion(name("a"), name("b")),
            ]),
            "a & b | a ! b",
        ),
        (
            Intersection(vec![
                Union(vec![name("a"), name("b")]),
                Intersection(vec![name("a"), name("b")]),
            ]),
            "(a | b) & (a & b)",
        ),
        (
            exclusion(
                exclusion(name("a"), name("b")),
                Intersection(vec![name("a"), name("b")]),
            ),
            "a ! b ! (a & b)",
        ),
        (
            exclusion(
                Union(vec![name("a"), name("b")]),
                exclusion(name("a"), name("b")),
            ),
            "(a | b) ! (a ! b)",
        ),
    ];
    for (tree, expected) in cases {
        assert_eq!(printed_expression(tree), expected);
    }
}

#[test]
fn errors_at_the_cap_are_all_reported() {
    let relations: Vec<_> = (0..MAX_ERRORS).map(|i| format!(r#""r{i}": "|""#)).collect();
    let errors = refused(&format!(r#"{{ "t": {{ {} }} }}"#, relations.join(", ")));
    assert_eq!(errors.len(), MAX_ERRORS);
    assert!(
        errors
            .iter()
            .all(|error| matches!(error.kind, ErrorKind::Syntax { .. }))
    );
}

#[test]
fn closed_groups_leave_the_nesting_depth() {
    let groups = vec!["(a)"; MAX_REWRITE_DEPTH + 1].join(" | ");
    assert_eq!(
        rewrite(&groups),
        Union(vec![name("a"); MAX_REWRITE_DEPTH + 1])
    );
}

// Limits.

#[test]
fn each_limit_holds_at_its_value_and_breaks_one_past_it() {
    let at = "n".repeat(MAX_NAME_BYTES);
    let past = "n".repeat(MAX_NAME_BYTES + 1);
    parsed(&format!(r#"{{ "{at}": {{ "{at}": "this" }} }}"#));
    for json in [
        format!(r#"{{ "t": {{ "{past}": "this" }} }}"#),
        format!(r#"{{ "{past}": {{ "a": "this" }} }}"#),
    ] {
        assert!(
            matches!(only(&json).kind, ErrorKind::Name(NameError::TooLong(_))),
            "{json}"
        );
    }
    let dotted = format!("{}.{}", "a".repeat(31), "b".repeat(32));
    assert_eq!(dotted.len(), MAX_NAME_BYTES);
    parsed(&format!(r#"{{ "{dotted}": {{ "a": "this" }} }}"#));

    let expression = |bytes: usize| format!("this{}", " ".repeat(bytes - 4));
    assert_eq!(rewrite(&expression(MAX_EXPRESSION_BYTES)), This);
    assert!(matches!(
        syntax(&expression(MAX_EXPRESSION_BYTES + 1)).1,
        ErrorKind::ExpressionTooLong
    ));

    assert_eq!(
        parsed(&theory_of(MAX_RELATIONS, |_| "this".to_owned()))
            .relations
            .relations()
            .count(),
        MAX_RELATIONS
    );
    assert!(matches!(
        only(&theory_of(MAX_RELATIONS + 1, |_| "this".to_owned())).kind,
        ErrorKind::TooManyRelations(501)
    ));

    assert_eq!(MAX_DOCUMENT_BYTES, 4 * 1024 * 1024);
    let document = |bytes: usize| {
        let document = r#"{ "t": { "a": "this" } }"#;
        format!("{document}{}", " ".repeat(bytes - document.len()))
    };
    parsed(&document(MAX_DOCUMENT_BYTES));
    assert!(matches!(
        only(&document(MAX_DOCUMENT_BYTES + 1)).kind,
        ErrorKind::TooLarge
    ));
}

/// A theory `t` of relations `r0`, `r1` and on, each mapped by `expression`.
fn theory_of(count: usize, expression: impl Fn(usize) -> String) -> String {
    let relations: Vec<_> = (0..count)
        .map(|i| format!(r#""r{i}": "{}""#, expression(i)))
        .collect();
    format!(r#"{{ "t": {{ {} }} }}"#, relations.join(", "))
}

// Names.

#[test]
fn a_dotted_theory_name_needs_a_name_between_each_dot() {
    for theory in [".a", "a.", "a..b", "a.1b"] {
        let error = only(&format!(r#"{{ "{theory}": {{ "a": "this" }} }}"#));
        assert!(
            matches!(error.kind, ErrorKind::Name(NameError::Invalid(_))),
            "{theory}"
        );
    }
}

#[test]
fn names_are_case_sensitive() {
    let document = parsed(r#"{ "t": { "owner": "this", "Owner": "owner" } }"#);
    assert_eq!(listed(&document)[1], (relation("Owner"), name("owner")));
}

// Checks.

#[test]
fn a_dangling_target_is_reported_for_each_relation_that_names_it() {
    let errors = refused(r#"{ "t": { "a": "x", "b": "this | x" } }"#);
    let relations: Vec<_> = errors.iter().map(|e| e.relation.as_deref()).collect();
    assert_eq!(relations, [Some("a"), Some("b")]);
}

// Printing.

#[test]
fn every_fixture_theory_is_in_canonical_form() {
    for text in [
        include_str!("../../sungma/tests/fixtures/theories/file.json"),
        include_str!("../../sungma/tests/fixtures/theories/folder.json"),
        include_str!("../../sungma/tests/fixtures/theories/group.json"),
    ] {
        let document = parsed(text);
        assert_eq!(theory::print(&document.theory, &document.relations), text);
    }
}

#[test]
fn key_order_doesnt_change_the_printed_form() {
    let one = parsed(r#"{ "t": { "b": "this", "a": "b", "c": "a & b" } }"#);
    let other = parsed(r#"{ "t": { "c": "a & b", "a": "b", "b": "this" } }"#);
    assert_eq!(
        theory::print(&t(), &one.relations),
        theory::print(&t(), &other.relations)
    );
}

#[test]
fn the_printer_refuses_caller_defects() {
    let defects: Vec<(&str, Relations)> = vec![
        ("t", vec![("this", This)]),
        ("t", vec![("This", This)]),
        ("t", vec![("THIS", This), ("r", Computed("THIS"))]),
        (
            "t",
            vec![
                ("this", This),
                (
                    "r",
                    FactTo {
                        factset: "this",
                        computed: "r",
                    },
                ),
            ],
        ),
        ("t", vec![("r", Union(vec![This]))]),
        ("t", vec![("r", Intersection(vec![This]))]),
        ("t", vec![("r", exclusion(This, Intersection(vec![This])))]),
        ("t", vec![("r-s", This)]),
        ("t", vec![("a b", This), ("r", Computed("a b"))]),
        ("t.", vec![("r", This)]),
    ];
    for (theory, relations) in defects {
        let checked = checked(&relations);
        let theory = TheoryName::new_unchecked(theory);
        let printed = panic::catch_unwind(AssertUnwindSafe(|| theory::print(&theory, &checked)));
        assert!(printed.is_err(), "printed {theory}: {relations:?}");
    }
}

// Round trips.

const NAMES: [&str; 4] = ["r0", "r1", "r2", "r3"];

/// A leaf for relation `at`. Computed subjectsets name only later relations,
/// so a theory of them has no cycle.
fn leaf(at: usize) -> BoxedStrategy<Rewrite<RelationName>> {
    let fact_to = (0..NAMES.len(), 0..NAMES.len()).prop_map(|(f, c)| fact_to(NAMES[f], NAMES[c]));
    if at + 1 == NAMES.len() {
        prop_oneof![Just(This), fact_to].boxed()
    } else {
        let later = proptest::sample::select(NAMES[at + 1..].to_vec()).prop_map(name);
        prop_oneof![Just(This), fact_to, later].boxed()
    }
}

/// Operators have two or more operands, as the grammar writes them.
fn small(at: usize) -> BoxedStrategy<Rewrite<RelationName>> {
    leaf(at)
        .prop_recursive(4, 24, 3, |inner| {
            prop_oneof![
                vec(inner.clone(), 2..4).prop_map(Union),
                vec(inner.clone(), 2..4).prop_map(Intersection),
                (inner.clone(), inner).prop_map(|(base, excluded)| exclusion(base, excluded)),
            ]
        })
        .boxed()
}

/// A spine of operators down to one leaf, up to the depth limit, with a
/// leaf beside each step. A run of unions in unions nests parentheses up to
/// the grouping limit.
fn deep(at: usize) -> BoxedStrategy<Rewrite<RelationName>> {
    let levels = prop_oneof![Just(MAX_REWRITE_DEPTH - 1), 0..MAX_REWRITE_DEPTH,];
    (leaf(at), levels)
        .prop_flat_map(move |(first, levels)| {
            vec((0..4u8, leaf(at)), levels).prop_map(move |spine| {
                spine
                    .into_iter()
                    .fold(first.clone(), |inner, (kind, beside)| match kind {
                        0 => Union(vec![inner, beside]),
                        1 => Intersection(vec![inner, beside]),
                        2 => exclusion(inner, beside),
                        _ => exclusion(beside, inner),
                    })
            })
        })
        .boxed()
}

/// One wide run of a single operator.
fn wide(at: usize) -> BoxedStrategy<Rewrite<RelationName>> {
    (any::<bool>(), vec(leaf(at), 2..300))
        .prop_map(|(union, operands)| {
            if union {
                Union(operands)
            } else {
                Intersection(operands)
            }
        })
        .boxed()
}

fn tree(at: usize) -> BoxedStrategy<Rewrite<RelationName>> {
    prop_oneof![small(at), deep(at), wide(at)].boxed()
}

fn theory() -> impl Strategy<Value = Vec<(RelationName, Rewrite<RelationName>)>> {
    (tree(0), tree(1), tree(2), tree(3)).prop_map(|(a, b, c, d)| {
        NAMES
            .iter()
            .map(|name| relation(name))
            .zip([a, b, c, d])
            .collect()
    })
}

const TOKENS: [&str; 11] = ["this", "r0", "r1", "r2", "(", ")", ",", "|", "&", "!", " "];

proptest! {
    /// Printing a theory and parsing it back gives an equal theory, and
    /// printing that gives the same text.
    #[test]
    fn a_printed_theory_parses_back_equal(relations in theory()) {
        let checked = Theory::new(relations.clone()).expect("a valid theory");
        let text = theory::print(&t(), &checked);
        let document = theory::parse(&text)
            .map_err(|errors| TestCaseError::fail(format!("{text}\n{errors:?}")))?;
        prop_assert_eq!(listed(&document), relations);
        prop_assert_eq!(theory::print(&t(), &document.relations), text);
    }

    /// Expression text built from the grammar's tokens either is refused, or
    /// parses to a tree that prints and parses back equal.
    #[test]
    fn expression_text_parses_print_and_parse_alike(
        tokens in vec(proptest::sample::select(TOKENS.to_vec()), 1..40),
    ) {
        let expression = tokens.concat();
        let json = format!(
            r#"{{ "t": {{ "r": "{expression}", "r0": "this", "r1": "this", "r2": "this" }} }}"#
        );
        if let Ok(document) = theory::parse(&json) {
            let text = theory::print(&t(), &document.relations);
            let again = theory::parse(&text)
                .map_err(|errors| TestCaseError::fail(format!("{text}\n{errors:?}")))?;
            prop_assert_eq!(listed(&again), listed(&document));
            prop_assert_eq!(theory::print(&t(), &again.relations), text);
        }
    }

    /// The parser never panics, whatever text it is given.
    #[test]
    fn any_text_is_parsed_or_refused(text in any::<String>()) {
        let _ = theory::parse(&text);
    }

    /// The parser never panics on near-JSON text, or on any expression.
    #[test]
    fn any_near_json_is_parsed_or_refused(
        text in r#"[{}\[\]":, tThisr0-9_.|&!()\\]{0,200}"#,
        expression in any::<String>(),
    ) {
        let _ = theory::parse(&text);
        let json = format!(r#"{{ "t": {{ "r": {} }} }}"#, serde_json::to_string(&expression).unwrap());
        let _ = theory::parse(&json);
    }
}

// Hostile input.

/// Runs `work`, failing if it takes longer than a bound that only
/// quadratic work, or worse, would pass. Only a release build is timed: a
/// debug build on a slow runner can be ten times slower.
fn within<T>(work: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let result = work();
    let elapsed = start.elapsed();
    assert!(
        cfg!(debug_assertions) || elapsed < Duration::from_secs(5),
        "took {elapsed:?}"
    );
    result
}

#[test]
fn the_longest_chain_of_relations_is_checked_quickly() {
    let chain = theory_of(MAX_RELATIONS, |i| format!("r{}", i + 1));
    let errors = within(|| refused(&chain));
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].relation.as_deref(), Some("r499"));
}

#[test]
fn the_largest_set_of_relations_reaching_each_other_is_one_cycle() {
    let count = MAX_RELATIONS;
    let ring = theory_of(count, |i| {
        format!("r{} | r{}", (i + 1) % count, (i + 7) % count)
    });
    let error = within(|| only(&ring));
    let ErrorKind::Theory(TheoryError::RewriteCycle { path, .. }) = error.kind else {
        panic!("expected a cycle, got {error}");
    };
    assert_eq!(path.first(), path.last());
    assert_eq!(error.relation.as_deref(), Some("r0"));
}

#[test]
fn far_more_relations_than_the_limit_are_refused_quickly() {
    let count = 200_000;
    let many = theory_of(count, |_| "this".to_owned());
    assert!(many.len() <= MAX_DOCUMENT_BYTES);
    let error = within(|| only(&many));
    assert!(matches!(error.kind, ErrorKind::TooManyRelations(200_000)));
}

// Text and encoding.

#[test]
fn columns_count_characters() {
    let (column, kind) = syntax("a | 😀");
    assert_eq!(column, Some(5));
    assert_eq!(
        kind.to_string(),
        "expected 'this', a relation name or '(', found '😀'"
    );
    let (column, _) = syntax("((a) | b");
    assert_eq!(column, Some(9));
}

#[test]
fn nul_is_refused() {
    let raw = format!(r#"{{ "t": {{ "a": "this{}" }} }}"#, char::from(0));
    assert!(matches!(only(&raw).kind, ErrorKind::Json(_)));
    let escaped = format!(r#"{{ "t": {{ "a": "this{}u0000" }} }}"#, '\\');
    assert!(matches!(only(&escaped).kind, ErrorKind::Escape(_)));
}

#[test]
fn a_byte_order_mark_is_refused() {
    let bom = char::from_u32(0xFEFF).unwrap();
    let error = only(&format!(r#"{bom}{{ "t": {{ "a": "this" }} }}"#));
    assert!(matches!(error.kind, ErrorKind::Json(_)), "{error}");
}

#[test]
fn deeply_nested_json_is_an_error_not_a_crash() {
    let nested = |open: &str, close: &str| {
        let value = format!("{}1{}", open.repeat(10_000), close.repeat(10_000));
        only(&format!(r#"{{ "t": {{ "a": {value} }} }}"#)).kind
    };
    // A relation's value is read no further than its type, so a deep array
    // or object is skipped unbuilt.
    assert!(matches!(nested("[", "]"), ErrorKind::NotAnExpression));
    assert!(matches!(
        nested(r#"{"a":"#, "}"),
        ErrorKind::NotAnExpression
    ));
}

// Determinism.

#[test]
fn a_refused_document_reports_the_same_errors_each_time() {
    let json = r#"{ "t": { "a": "x | y", "b": "c", "c": "b", "d": "(", "e": "z & x" } }"#;
    let messages = || -> Vec<String> { refused(json).iter().map(ToString::to_string).collect() };
    let first = messages();
    for _ in 0..20 {
        assert_eq!(messages(), first);
    }
}

// Quoting.

#[test]
fn an_error_quotes_at_most_a_bounded_cut_of_the_input() {
    let long = "n".repeat(1000);
    let error = only(&format!(r#"{{ "t": {{ "{long}": "this" }} }}"#));
    let quoted = format!("{}...", "n".repeat(MAX_QUOTE_BYTES));
    assert_eq!(error.relation.as_deref(), Some(quoted.as_str()));
    assert!(matches!(&error.kind, ErrorKind::Name(NameError::TooLong(name)) if *name == quoted));
    // A cut never splits a character: 21 three-byte characters fill 63 bytes.
    let wide = "€".repeat(30);
    let error = only(&format!(r#"{{ "t": {{ "{wide}": "this" }} }}"#));
    let ErrorKind::Name(NameError::TooLong(name)) = &error.kind else {
        panic!("expected a name too long, got {error}");
    };
    assert_eq!(*name, format!("{}...", "€".repeat(21)));
    let target = "x".repeat(200);
    let error = only(&format!(r#"{{ "t": {{ "a": "{target}" }} }}"#));
    assert!(error.to_string().len() < 300, "{error}");
}

#[test]
fn an_error_escapes_hidden_characters() {
    // JSON refuses C0 controls in a string, so these are DEL, a C1 control,
    // zero-width, reordering and invisible characters.
    for code in [0x7f, 0x9b, 0x200b, 0x202e, 0xfeff, 0x2066] {
        let c = char::from_u32(code).unwrap();
        let errors = [
            only(&format!(r#"{{ "t": {{ "a{c}b": "this" }} }}"#)),
            only(&format!(r#"{{ "t": {{ "a": "a | {c}" }} }}"#)),
            only(&format!(r#"{{ "t{c}": {{ "a": "this" }} }}"#)),
        ];
        for error in errors {
            let message = error.to_string();
            assert!(!message.contains(c), "{code:x}: {message}");
            assert!(
                message.contains(&format!("u{{{code:04x}}}")),
                "{code:x}: {message}"
            );
        }
    }
}

#[test]
fn an_escape_error_quotes_the_decoded_text_escaped() {
    let backslash = char::from(92);
    let error = only(&format!(r#"{{ "t": {{ "a{backslash}u202eb": "this" }} }}"#));
    assert!(matches!(&error.kind, ErrorKind::Escape(_)));
    let message = error.to_string();
    assert!(
        !message.contains(char::from_u32(0x202e).unwrap()),
        "{message}"
    );
    assert!(message.contains("u{202e}"), "{message}");
}

// The grammar, token by token.

/// Relations every grammar case may name, each declared as `this`.
const DECLARED: [&str; 12] = [
    "a", "b", "c", "d", "p", "_a", "a1", "_", "thisone", "this_", "_this", "THIS1",
];

/// A document whose relation `r` has `expression`, beside every relation in
/// [`DECLARED`].
fn grammar_case(expression: &str) -> String {
    let declared: Vec<_> = DECLARED
        .iter()
        .map(|name| format!(r#""{name}": "this""#))
        .collect();
    format!(
        r#"{{ "t": {{ "r": "{expression}", {} }} }}"#,
        declared.join(", ")
    )
}

fn union(operands: Vec<Rewrite<RelationName>>) -> Rewrite<RelationName> {
    Union(operands)
}

fn intersection(operands: Vec<Rewrite<RelationName>>) -> Rewrite<RelationName> {
    Intersection(operands)
}

#[test]
fn every_valid_form_parses_to_its_tree_and_prints_canonically() {
    let (a, b, c, d) = (name("a"), name("b"), name("c"), name("d"));
    let cases = [
        ("this", This, "this"),
        ("a", a.clone(), "a"),
        ("(p, a)", fact_to("p", "a"), "(p, a)"),
        ("(p,a)", fact_to("p", "a"), "(p, a)"),
        ("( p , a )", fact_to("p", "a"), "(p, a)"),
        ("((p, a))", fact_to("p", "a"), "(p, a)"),
        (
            "(p, undeclared)",
            fact_to("p", "undeclared"),
            "(p, undeclared)",
        ),
        ("(this)", This, "this"),
        ("((((a))))", a.clone(), "a"),
        ("this ! this", exclusion(This, This), "this ! this"),
        ("this & this", intersection(vec![This, This]), "this & this"),
        ("this|this", union(vec![This, This]), "this | this"),
        ("a|b", union(vec![a.clone(), b.clone()]), "a | b"),
        ("(a)|(b)", union(vec![a.clone(), b.clone()]), "a | b"),
        (
            "a | b & c | d",
            union(vec![
                a.clone(),
                intersection(vec![b.clone(), c.clone()]),
                d.clone(),
            ]),
            "a | b & c | d",
        ),
        (
            "a & b ! c & d",
            intersection(vec![a.clone(), exclusion(b.clone(), c.clone()), d.clone()]),
            "a & b ! c & d",
        ),
        (
            "a ! b & c",
            intersection(vec![exclusion(a.clone(), b.clone()), c.clone()]),
            "a ! b & c",
        ),
        (
            "a ! (b ! c)",
            exclusion(a.clone(), exclusion(b.clone(), c.clone())),
            "a ! (b ! c)",
        ),
        (
            "(a ! b) ! c",
            exclusion(exclusion(a.clone(), b.clone()), c.clone()),
            "a ! b ! c",
        ),
        (
            "(a | b) & c",
            intersection(vec![union(vec![a.clone(), b.clone()]), c.clone()]),
            "(a | b) & c",
        ),
        (
            "a & (b | c)",
            intersection(vec![a.clone(), union(vec![b.clone(), c.clone()])]),
            "a & (b | c)",
        ),
        (
            "(a & b) | c",
            union(vec![intersection(vec![a.clone(), b.clone()]), c.clone()]),
            "a & b | c",
        ),
        (
            "a | (b | c)",
            union(vec![a.clone(), union(vec![b.clone(), c.clone()])]),
            "a | (b | c)",
        ),
        (
            "(a & b) & c",
            intersection(vec![intersection(vec![a.clone(), b.clone()]), c.clone()]),
            "(a & b) & c",
        ),
        (
            "a ! (b & c)",
            exclusion(a.clone(), intersection(vec![b.clone(), c.clone()])),
            "a ! (b & c)",
        ),
        (
            "(a | b) ! c",
            exclusion(union(vec![a.clone(), b.clone()]), c.clone()),
            "(a | b) ! c",
        ),
        (
            "(a ! b) & (c ! d)",
            intersection(vec![
                exclusion(a.clone(), b.clone()),
                exclusion(c.clone(), d.clone()),
            ]),
            "a ! b & c ! d",
        ),
        (
            "(p, a) ! (p, b)",
            exclusion(fact_to("p", "a"), fact_to("p", "b")),
            "(p, a) ! (p, b)",
        ),
        (
            "a ! (p, b)",
            exclusion(a.clone(), fact_to("p", "b")),
            "a ! (p, b)",
        ),
        (
            "(p, a) & (p, b) | this",
            union(vec![
                intersection(vec![fact_to("p", "a"), fact_to("p", "b")]),
                This,
            ]),
            "(p, a) & (p, b) | this",
        ),
        (
            "a ! b ! c & d ! a | b",
            union(vec![
                intersection(vec![
                    exclusion(exclusion(a.clone(), b.clone()), c.clone()),
                    exclusion(d.clone(), a.clone()),
                ]),
                b.clone(),
            ]),
            "a ! b ! c & d ! a | b",
        ),
        (
            "_a | a1 | _ | thisone | this_ | _this | THIS1",
            union(
                ["_a", "a1", "_", "thisone", "this_", "_this", "THIS1"]
                    .into_iter()
                    .map(name)
                    .collect(),
            ),
            "_a | a1 | _ | thisone | this_ | _this | THIS1",
        ),
    ];
    for (expression, tree, canonical) in cases {
        let document = parsed(&grammar_case(expression));
        assert_eq!(listed(&document)[0].1, tree, "{expression}");
        assert_eq!(printed_expression(tree.clone()), canonical, "{expression}");
        let again = parsed(&grammar_case(canonical));
        assert_eq!(listed(&again)[0].1, tree, "{expression} as {canonical}");
    }
}

#[test]
fn every_invalid_form_is_refused_where_it_goes_wrong() {
    const TERM: &str = "'this', a relation name or '('";
    const END: &str = "an operator or the end";
    const CLOSE: &str = "')'";
    const RELATION: &str = "a relation name";
    let cases = [
        ("   ", 4, "the end", TERM),
        ("| a", 1, "'|'", TERM),
        ("& a", 1, "'&'", TERM),
        ("! a", 1, "'!'", TERM),
        ("a !", 4, "the end", TERM),
        ("a &", 4, "the end", TERM),
        ("a || b", 4, "'|'", TERM),
        ("a !! b", 4, "'!'", TERM),
        ("a & | b", 5, "'|'", TERM),
        ("a ! & b", 5, "'&'", TERM),
        ("a)", 2, "')'", END),
        (")a", 1, "')'", TERM),
        ("()", 2, "')'", TERM),
        ("((a)", 5, "the end", CLOSE),
        ("(a))", 4, "')'", END),
        ("(a)(b)", 4, "'('", END),
        ("a (b)", 3, "'('", END),
        ("this(a)", 5, "'('", END),
        ("a b", 3, "'b'", END),
        ("this this", 6, "'this'", END),
        ("a,b", 2, "','", END),
        ("(,a)", 2, "','", TERM),
        ("(a,)", 4, "')'", RELATION),
        ("(a, b, c)", 6, "','", CLOSE),
        ("(a b, c)", 4, "'b'", CLOSE),
        ("(a | b, c)", 7, "','", CLOSE),
        ("(a, (p, b))", 5, "'('", RELATION),
        ("(this, a)", 2, "'this'", RELATION),
        ("(a, this)", 5, "'this'", RELATION),
        ("(p, a", 6, "the end", CLOSE),
        ("a.b", 2, "'.'", END),
        ("1a", 1, "'1'", TERM),
        ("a#b", 2, "'#'", END),
        ("a:b", 2, "':'", END),
        ("a-b", 2, "'-'", END),
        ("a | b ; c", 7, "';'", END),
        ("a ~ b", 3, "'~'", END),
        ("a | b/", 6, "'/'", END),
    ];
    for (expression, column, found, expected) in cases {
        let error = only(&grammar_case(expression));
        assert_eq!(error.relation.as_deref(), Some("r"), "{expression}");
        assert_eq!(error.column, Some(column), "{expression}");
        assert_eq!(
            error.kind.to_string(),
            format!("expected {expected}, found {found}"),
            "{expression}"
        );
    }
}

#[test]
fn a_reserved_or_overlong_name_is_refused_where_it_appears() {
    let long = "n".repeat(MAX_NAME_BYTES + 1);
    let fact_to_long = format!("a & (p, {long})");
    let cases = [
        ("THIS", 1),
        ("a | This", 5),
        ("(p, tHIS)", 5),
        ("(THIS, a)", 2),
        (long.as_str(), 1),
        (fact_to_long.as_str(), 9),
    ];
    for (expression, column) in cases {
        let error = only(&grammar_case(expression));
        assert_eq!(error.column, Some(column), "{expression}");
        assert!(
            matches!(
                error.kind,
                ErrorKind::Name(NameError::Reserved(_) | NameError::TooLong(_))
            ),
            "{expression}: {error}"
        );
    }
}

#[test]
fn the_relation_limit_counts_distinct_relations() {
    let mut document = theory_of(MAX_RELATIONS, |_| "this".to_owned());
    document.insert_str(document.len() - 4, r#", "r0": "this""#);
    let error = only(&document);
    assert_eq!(error.relation.as_deref(), Some("r0"));
    assert!(matches!(
        error.kind,
        ErrorKind::Theory(TheoryError::DuplicateRelation(_))
    ));
}
