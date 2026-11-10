//! Namespace documents: parsed, checked whole, and printed back.

mod grammar;

use proptest::{collection::vec, prelude::*};
use sungma::{
    name::{NameError, NamespaceName, RelationName},
    namespace::{Constraint, MAX_REWRITE_DEPTH, Namespace, NamespaceError},
    non_empty::NonEmpty,
    nonempty,
    rewrite::Rewrite::{self, Computed, Exclusion, FactTo, Intersection, This, Union},
};
use sungma_lang::namespace::{
    self, DocumentError, ErrorKind, MAX_DOCUMENT_BYTES, MAX_ERRORS, MAX_EXPRESSION_BYTES,
    MAX_NAMESPACE_BYTES, MAX_QUOTE_BYTES, MAX_RELATION_BYTES, MAX_RELATIONS, Named,
    NamedConstraint, NamespaceDocument, PrintError,
};

fn parsed(json: &str) -> NamespaceDocument {
    namespace::parse(json).unwrap_or_else(|errors| panic!("refused: {errors:?}"))
}

fn refused(json: &str) -> Vec<DocumentError> {
    match namespace::parse(json) {
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

/// Whether an expression's text holds the keyword `this`.
fn mentions_this(expression: &str) -> bool {
    expression
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(|word| word == "this")
}

/// A relation's value for `expression`: the constraints form, taking
/// identities, when it reads `this`, and the plain string otherwise.
fn value(expression: &str) -> String {
    if mentions_this(expression) {
        format!(r#"{{ "rewrite": "{expression}", "constraints": ["identity"] }}"#)
    } else {
        format!(r#""{expression}""#)
    }
}

/// Relations `a`, `b`, `c` and `p`, each taking identities.
const STORED: &str = r#""a": { "constraints": ["identity"] }, "b": { "constraints": ["identity"] }, "c": { "constraints": ["identity"] }, "p": { "constraints": ["identity"] }"#;

/// The rewrite of the one relation `r`.
fn rewrite(expression: &str) -> Rewrite<RelationName> {
    let json = format!(r#"{{ "t": {{ "r": {}, {STORED} }} }}"#, value(expression));
    let document = parsed(&json);
    listed(&document)[0].1.clone()
}

/// The error the expression of relation `r` is refused with, and its column.
fn syntax(expression: &str) -> (Option<usize>, ErrorKind) {
    let json = format!(
        r#"{{ "t": {{ "r": "{expression}", "a": {{ "constraints": ["identity"] }}, "b": {{ "constraints": ["identity"] }} }} }}"#
    );
    let error = only(&json);
    assert_eq!(error.relation.as_deref(), Some("r"));
    (error.column, error.kind)
}

fn relation(name: &str) -> RelationName {
    RelationName::new_unchecked(name)
}

/// The namespace `t`.
fn t() -> NamespaceName {
    NamespaceName::new_unchecked("t")
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

/// A namespace's relations, named as written.
type Relations = Vec<(&'static str, Rewrite<&'static str>)>;

/// Whether a rewrite reads `this`.
fn reads_this<R>(rewrite: &Rewrite<R>) -> bool {
    match rewrite {
        This => true,
        Computed(_) | FactTo { .. } => false,
        Union(operands) | Intersection(operands) => operands.iter().any(reads_this),
        Exclusion(base, excluded) => reads_this(base) || reads_this(excluded),
    }
}

/// Identities, when `rewrite` reads `this`, and nothing otherwise.
fn identities<R>(rewrite: &Rewrite<R>) -> Vec<NamedConstraint> {
    if reads_this(rewrite) {
        vec![Constraint::Identity]
    } else {
        vec![]
    }
}

/// A constraint as written, each name made with `new_unchecked`.
fn written(constraint: &str) -> NamedConstraint {
    match constraint.split_once('#') {
        _ if constraint == "identity" => Constraint::Identity,
        None => Constraint::Resource(NamespaceName::new_unchecked(constraint)),
        Some((namespace, set)) => {
            Constraint::Subjectset(NamespaceName::new_unchecked(namespace), relation(set))
        }
    }
}

/// A valid namespace of relations written with plain names, each made with
/// `new_unchecked`, each that reads `this` taking identities.
fn checked(relations: &[(&str, Rewrite<&str>)]) -> Named {
    let relations: Vec<_> = relations
        .iter()
        .map(|(name, rewrite)| (*name, rewrite.clone(), identities(rewrite)))
        .collect();
    checked_with(relations)
}

/// A valid namespace of relations and their constraints, each name made
/// with `new_unchecked`.
fn checked_with(relations: Vec<(&str, Rewrite<&str>, Vec<NamedConstraint>)>) -> Named {
    let relations = relations
        .into_iter()
        .map(|(name, rewrite, constraints)| {
            let rewrite = rewrite.map(&mut |name| relation(name));
            (relation(name), rewrite, constraints)
        })
        .collect();
    Namespace::new(relations).expect("a valid namespace")
}

fn print(namespace: &str, relations: &[(&str, Rewrite<&str>)]) -> String {
    namespace::print(
        &NamespaceName::new_unchecked(namespace),
        &checked(relations),
    )
    .expect("a printable namespace")
}

/// A document's relations, in document order.
fn listed(document: &NamespaceDocument) -> Vec<(RelationName, Rewrite<RelationName>)> {
    document
        .relations
        .relations()
        .map(|(name, rewrite)| (name.clone(), rewrite.clone()))
        .collect()
}

/// A namespace's relations with their constraints, sorted, in declaration
/// order.
fn declared(relations: &Named) -> Vec<(RelationName, Rewrite<RelationName>, Vec<NamedConstraint>)> {
    relations
        .relations()
        .map(|(name, rewrite)| {
            let mut constraints = relations.constraints(name).to_vec();
            constraints.sort();
            (name.clone(), rewrite.clone(), constraints)
        })
        .collect()
}

/// How the printer writes `tree` as the rewrite of relation `r`, beside
/// every relation in [`DECLARED`].
fn printed_expression(tree: Rewrite<RelationName>) -> String {
    let mut relations: Vec<_> = DECLARED
        .iter()
        .map(|name| (relation(name), This, vec![Constraint::Identity]))
        .collect();
    let constraints = identities(&tree);
    relations.push((relation("r"), tree, constraints));
    let namespace = Namespace::new(relations).expect("a valid namespace");
    let text = namespace::print(&t(), &namespace).expect("a printable namespace");
    let mut lines = text.lines();
    let line = lines
        .find(|line| line.starts_with("    \"r\": "))
        .expect("relation r is printed");
    if line.ends_with('{') {
        let next = lines.next().expect("relation r's object has a key");
        return match next.strip_prefix("      \"rewrite\": \"") {
            Some(rewrite) => rewrite
                .trim_end_matches(',')
                .trim_end_matches('"')
                .to_owned(),
            None => "this".to_owned(),
        };
    }
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
        "../../sungma/tests/fixtures/namespaces/file.json"
    ));
    assert_eq!(document.namespace.as_str(), "file");
    let viewer = &document
        .relations
        .relations()
        .find(|(r, _)| r.as_str() == "viewer")
        .unwrap()
        .1;
    assert_eq!(
        **viewer,
        exclusion(
            Union(nonempty![This, name("editor"), fact_to("parent", "viewer")]),
            name("banned")
        )
    );
}

#[test]
fn relations_keep_document_order() {
    let document = parsed(r#"{ "t": { "b": { "constraints": ["identity"] }, "a": "b" } }"#);
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
        Union(nonempty![
            name("a"),
            Intersection(nonempty![name("b"), exclusion(name("c"), name("p"))])
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
        Union(nonempty![name("a"), name("b"), name("c")])
    );
    assert_eq!(
        rewrite("a & b & c"),
        Intersection(nonempty![name("a"), name("b"), name("c")])
    );
}

#[test]
fn parentheses_are_kept_as_structure() {
    assert_eq!(
        rewrite("(a | b) | c"),
        Union(nonempty![Union(nonempty![name("a"), name("b")]), name("c")])
    );
    assert_eq!(rewrite("((a))"), name("a"));
}

#[test]
fn whitespace_only_separates_tokens() {
    assert_eq!(rewrite("(p,a)|this"), rewrite(" ( p , a )  |  this "));
    assert_eq!(
        rewrite("(p,a)|this"),
        Union(nonempty![fact_to("p", "a"), This])
    );
}

#[test]
fn a_name_is_the_longest_run_of_name_characters() {
    let document =
        parsed(r#"{ "t": { "thisone": { "constraints": ["identity"] }, "r": "thisone" } }"#);
    assert_eq!(listed(&document)[1].1, name("thisone"));
}

#[test]
fn a_dotted_namespace_name_is_one_name() {
    assert_eq!(
        parsed(r#"{ "drive.file": { "owner": { "constraints": ["identity"] } } }"#)
            .namespace
            .as_str(),
        "drive.file"
    );
}

#[test]
fn a_namespace_without_relations_is_empty() {
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
        let error = only(&format!(
            r#"{{ "t": {{ "{relation}": {{ "constraints": ["identity"] }} }} }}"#
        ));
        assert!(
            matches!(error.kind, ErrorKind::Name(NameError::Reserved(_))),
            "{relation}"
        );
    }
}

#[test]
fn names_follow_the_grammar() {
    for json in [
        r#"{ "t": { "1a": { "constraints": ["identity"] } } }"#,
        r#"{ "t": { "a-b": { "constraints": ["identity"] } } }"#,
        r#"{ "t": { "": { "constraints": ["identity"] } } }"#,
        r#"{ "t.": { "a": { "constraints": ["identity"] } } }"#,
        r#"{ "t:x": { "a": { "constraints": ["identity"] } } }"#,
    ] {
        assert!(
            matches!(only(json).kind, ErrorKind::Name(NameError::Invalid(_))),
            "{json}"
        );
    }
    let namespace = "n".repeat(MAX_NAMESPACE_BYTES + 1);
    let relation = "n".repeat(MAX_RELATION_BYTES + 1);
    for json in [
        format!(r#"{{ "t": {{ "{relation}": {{ "constraints": ["identity"] }} }} }}"#),
        format!(r#"{{ "{namespace}": {{ "a": {{ "constraints": ["identity"] }} }} }}"#),
    ] {
        assert!(
            matches!(only(&json).kind, ErrorKind::Name(NameError::TooLong(..))),
            "{json}"
        );
    }
    let namespace = "n".repeat(MAX_NAMESPACE_BYTES);
    let relation = "n".repeat(MAX_RELATION_BYTES);
    parsed(&format!(
        r#"{{ "{namespace}": {{ "{relation}": {{ "constraints": ["identity"] }} }} }}"#
    ));
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
fn a_document_declares_exactly_one_namespace() {
    assert!(matches!(only("{}").kind, ErrorKind::NoNamespace));
    let error = only(r#"{ "a": {}, "b": {} }"#);
    assert_eq!(
        error.to_string(),
        "a document declares one namespace, not 2"
    );
    let error = only(r#"{ "\u0061": {}, "b": {} }"#);
    assert_eq!(
        error.to_string(),
        "a document declares one namespace, not 2"
    );
    assert!(matches!(
        only(r#"{ "a": {}, "a": {} }"#).kind,
        ErrorKind::ManyNamespaces(2)
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
        r#"{ "t": { "\u0061": { "constraints": ["identity"] } } }"#,
        r#"{ "t": { "a": { "rewrite": "this |\nthis", "constraints": ["identity"] } } }"#,
        r#"{ "\u0074": { "a": { "constraints": ["identity"] } } }"#,
    ] {
        assert!(matches!(only(json).kind, ErrorKind::Escape(_)), "{json}");
    }
}

#[test]
fn a_relation_named_twice_is_one_error() {
    let error = only(
        r#"{ "t": { "a": { "constraints": ["identity"] }, "a": { "constraints": ["identity"] } } }"#,
    );
    assert!(matches!(
        error.kind,
        ErrorKind::Namespace(NamespaceError::DuplicateRelation(_))
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
    assert!(matches!(
        kind,
        ErrorKind::Namespace(NamespaceError::TooDeep)
    ));
    // Unions in groups: each level is a run, one level however wide.
    let unions =
        |levels: usize| (1..levels).fold("a".to_owned(), |inner, _| format!("(a | {inner})"));
    rewrite(&unions(MAX_REWRITE_DEPTH));
    assert!(matches!(
        syntax(&unions(MAX_REWRITE_DEPTH + 1)).1,
        ErrorKind::Namespace(NamespaceError::TooDeep)
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
        ErrorKind::Namespace(NamespaceError::RewriteCycle { path, .. }) if path == &["d", "e", "d"]
    ));
}

#[test]
fn a_dangling_target_is_reported_once_per_relation() {
    let error = only(r#"{ "t": { "a": "x | x & (x, b)" } }"#);
    assert_eq!(
        error.to_string(),
        "relation 'a': relation 'a' references 'x', which the namespace doesn't declare"
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
        ErrorKind::Namespace(NamespaceError::RewriteCycle { path, .. }) if path == ["a", "b", "a"]
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
    assert_eq!(only("{}").to_string(), "a document declares no namespace");
}

#[test]
fn the_printer_writes_the_canonical_form() {
    let people = || vec![written("identity"), written("group#member")];
    let relations = vec![
        (
            "viewer",
            exclusion(
                Union(nonempty![
                    This,
                    Computed("editor"),
                    FactTo {
                        factset: "parent",
                        computed: "viewer",
                    },
                ]),
                Computed("banned"),
            ),
            people(),
        ),
        (
            "editor",
            Union(nonempty![This, Computed("owner")]),
            people(),
        ),
        ("owner", This, vec![written("identity")]),
        (
            "parent",
            This,
            vec![
                written("identity"),
                written("folder#owner"),
                written("folder"),
            ],
        ),
        ("banned", This, vec![written("identity")]),
        (
            "auditor",
            Intersection(nonempty![This, Computed("viewer")]),
            vec![written("identity")],
        ),
    ];
    assert_eq!(
        namespace::print(
            &NamespaceName::new_unchecked("file"),
            &checked_with(relations)
        ),
        Ok(include_str!("../../sungma/tests/fixtures/namespaces/file.json").to_owned())
    );
    assert_eq!(print("t", &[]), "{\n  \"t\": {}\n}\n");
}

#[test]
fn the_printer_adds_only_the_parentheses_the_grammar_needs() {
    let cases = [
        (
            Union(nonempty![Union(nonempty![name("a"), name("b")]), name("c")]),
            "(a | b) | c",
        ),
        (
            Union(nonempty![
                Intersection(nonempty![name("a"), name("b")]),
                exclusion(name("a"), name("b")),
            ]),
            "a & b | a ! b",
        ),
        (
            Intersection(nonempty![
                Union(nonempty![name("a"), name("b")]),
                Intersection(nonempty![name("a"), name("b")]),
            ]),
            "(a | b) & (a & b)",
        ),
        (
            exclusion(
                exclusion(name("a"), name("b")),
                Intersection(nonempty![name("a"), name("b")]),
            ),
            "a ! b ! (a & b)",
        ),
        (
            exclusion(
                Union(nonempty![name("a"), name("b")]),
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
        Union(nonempty![name("a"); MAX_REWRITE_DEPTH + 1])
    );
}

// Limits.

#[test]
fn each_limit_holds_at_its_value_and_breaks_one_past_it() {
    let namespace = |bytes: usize| "n".repeat(bytes);
    let relation = |bytes: usize| "r".repeat(bytes);
    parsed(&format!(
        r#"{{ "{}": {{ "{}": {{ "constraints": ["identity"] }} }} }}"#,
        namespace(MAX_NAMESPACE_BYTES),
        relation(MAX_RELATION_BYTES)
    ));
    for (json, max) in [
        (
            format!(
                r#"{{ "t": {{ "{}": {{ "constraints": ["identity"] }} }} }}"#,
                relation(MAX_RELATION_BYTES + 1)
            ),
            MAX_RELATION_BYTES,
        ),
        (
            format!(
                r#"{{ "{}": {{ "a": {{ "constraints": ["identity"] }} }} }}"#,
                namespace(MAX_NAMESPACE_BYTES + 1)
            ),
            MAX_NAMESPACE_BYTES,
        ),
    ] {
        assert!(
            matches!(only(&json).kind, ErrorKind::Name(NameError::TooLong(_, at)) if at == max),
            "{json}"
        );
    }
    let dotted = format!("{}.{}", "a".repeat(127), "b".repeat(128));
    assert_eq!(dotted.len(), MAX_NAMESPACE_BYTES);
    parsed(&format!(
        r#"{{ "{dotted}": {{ "a": {{ "constraints": ["identity"] }} }} }}"#
    ));

    let expression = |bytes: usize| format!("this{}", " ".repeat(bytes - 4));
    assert_eq!(rewrite(&expression(MAX_EXPRESSION_BYTES)), This);
    assert!(matches!(
        syntax(&expression(MAX_EXPRESSION_BYTES + 1)).1,
        ErrorKind::ExpressionTooLong
    ));

    assert_eq!(
        parsed(&namespace_of(MAX_RELATIONS, |_| "this".to_owned()))
            .relations
            .relations()
            .count(),
        MAX_RELATIONS
    );
    assert!(matches!(
        only(&namespace_of(MAX_RELATIONS + 1, |_| "this".to_owned())).kind,
        ErrorKind::TooManyRelations(501)
    ));

    assert_eq!(MAX_DOCUMENT_BYTES, 4 * 1024 * 1024);
    let document = |bytes: usize| {
        let document = r#"{ "t": { "a": { "constraints": ["identity"] } } }"#;
        format!("{document}{}", " ".repeat(bytes - document.len()))
    };
    parsed(&document(MAX_DOCUMENT_BYTES));
    assert!(matches!(
        only(&document(MAX_DOCUMENT_BYTES + 1)).kind,
        ErrorKind::TooLarge
    ));
}

/// A namespace `t` of relations `r0`, `r1` and on, each mapped by `expression`.
fn namespace_of(count: usize, expression: impl Fn(usize) -> String) -> String {
    let relations: Vec<_> = (0..count)
        .map(|i| format!(r#""r{i}": {}"#, value(&expression(i))))
        .collect();
    format!(r#"{{ "t": {{ {} }} }}"#, relations.join(", "))
}

// Names.

#[test]
fn a_dotted_namespace_name_needs_a_name_between_each_dot() {
    for namespace in [".a", "a.", "a..b", "a.1b"] {
        let error = only(&format!(
            r#"{{ "{namespace}": {{ "a": {{ "constraints": ["identity"] }} }} }}"#
        ));
        assert!(
            matches!(error.kind, ErrorKind::Name(NameError::Invalid(_))),
            "{namespace}"
        );
    }
}

#[test]
fn names_are_case_sensitive() {
    let document =
        parsed(r#"{ "t": { "owner": { "constraints": ["identity"] }, "Owner": "owner" } }"#);
    assert_eq!(listed(&document)[1], (relation("Owner"), name("owner")));
}

// Checks.

#[test]
fn a_dangling_target_is_reported_for_each_relation_that_names_it() {
    let errors = refused(
        r#"{ "t": { "a": "x", "b": { "rewrite": "this | x", "constraints": ["identity"] } } }"#,
    );
    let relations: Vec<_> = errors.iter().map(|e| e.relation.as_deref()).collect();
    assert_eq!(relations, [Some("a"), Some("b")]);
}

// Printing.

#[test]
fn every_fixture_namespace_is_in_canonical_form() {
    for text in [
        include_str!("../../sungma/tests/fixtures/namespaces/file.json"),
        include_str!("../../sungma/tests/fixtures/namespaces/folder.json"),
        include_str!("../../sungma/tests/fixtures/namespaces/group.json"),
    ] {
        let document = parsed(text);
        assert_eq!(
            namespace::print(&document.namespace, &document.relations),
            Ok(text.to_owned())
        );
    }
}

#[test]
fn key_order_doesnt_change_the_printed_form() {
    let one =
        parsed(r#"{ "t": { "b": { "constraints": ["identity"] }, "a": "b", "c": "a & b" } }"#);
    let other =
        parsed(r#"{ "t": { "c": "a & b", "a": "b", "b": { "constraints": ["identity"] } } }"#);
    assert_eq!(
        namespace::print(&t(), &one.relations),
        namespace::print(&t(), &other.relations)
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
        ("t", vec![("r", Union(nonempty![This]))]),
        ("t", vec![("r", Intersection(nonempty![This]))]),
        (
            "t",
            vec![("r", exclusion(This, Intersection(nonempty![This])))],
        ),
        ("t", vec![("r-s", This)]),
        ("t", vec![("a b", This), ("r", Computed("a b"))]),
        ("t.", vec![("r", This)]),
    ];
    for (namespace, relations) in defects {
        let checked = checked(&relations);
        let namespace = NamespaceName::new_unchecked(namespace);
        let printed = namespace::print(&namespace, &checked);
        assert!(printed.is_err(), "printed {namespace}: {relations:?}");
    }
    let one = checked(&[("r", Union(nonempty![This]))]);
    assert_eq!(namespace::print(&t(), &one), Err(PrintError::OneOperand));
    let unchecked = checked(&[("this", This)]);
    assert!(matches!(
        namespace::print(&t(), &unchecked),
        Err(PrintError::Name(_))
    ));
    for constraint in [
        "identity#member",
        "a b",
        "t#this",
        "t#a b",
        "1t",
        "t#theory",
    ] {
        let unchecked = checked_with(vec![("r", This, vec![written(constraint)])]);
        assert!(
            matches!(namespace::print(&t(), &unchecked), Err(PrintError::Name(_))),
            "{constraint}"
        );
    }
    let identity = Constraint::Resource(NamespaceName::new_unchecked("identity"));
    let unchecked = checked_with(vec![("r", This, vec![identity])]);
    assert!(matches!(
        namespace::print(&t(), &unchecked),
        Err(PrintError::Name(_))
    ));
    let named_identity = checked(&[("r", This)]);
    let identity = NamespaceName::new_unchecked("identity");
    assert!(matches!(
        namespace::print(&identity, &named_identity),
        Err(PrintError::Name(_))
    ));
}

// Round trips.

const NAMES: [&str; 4] = ["r0", "r1", "r2", "r3"];

/// A leaf for relation `at`. Computed subjectsets name only later relations,
/// so a namespace of them has no cycle.
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
                vec(inner.clone(), 2..4).prop_map(union),
                vec(inner.clone(), 2..4).prop_map(intersection),
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
                        0 => Union(nonempty![inner, beside]),
                        1 => Intersection(nonempty![inner, beside]),
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
                self::union(operands)
            } else {
                intersection(operands)
            }
        })
        .boxed()
}

fn tree(at: usize) -> BoxedStrategy<Rewrite<RelationName>> {
    prop_oneof![small(at), deep(at), wide(at)].boxed()
}

/// Constraints of every kind: identities, resources of a plain or dotted
/// namespace, and subjectsets of another namespace or of `t` itself.
const KINDS: [&str; 6] = [
    "identity",
    "folder",
    "drive.file",
    "group#member",
    "t#r1",
    "t#r3",
];

/// One to four distinct constraints, in any order.
fn constraints() -> impl Strategy<Value = Vec<NamedConstraint>> {
    proptest::sample::subsequence(KINDS.to_vec(), 1..5)
        .prop_shuffle()
        .prop_map(|kinds| kinds.into_iter().map(written).collect())
}

type Generated = Vec<(RelationName, Rewrite<RelationName>, Vec<NamedConstraint>)>;

fn namespace() -> impl Strategy<Value = Generated> {
    let declared = |at| {
        (tree(at), constraints()).prop_map(|(tree, constraints)| {
            let constraints = if reads_this(&tree) {
                constraints
            } else {
                vec![]
            };
            (tree, constraints)
        })
    };
    (declared(0), declared(1), declared(2), declared(3)).prop_map(|(a, b, c, d)| {
        NAMES
            .iter()
            .map(|name| relation(name))
            .zip([a, b, c, d])
            .map(|(name, (tree, constraints))| (name, tree, constraints))
            .collect()
    })
}

const TOKENS: [&str; 11] = ["this", "r0", "r1", "r2", "(", ")", ",", "|", "&", "!", " "];

proptest! {
    /// Printing a namespace and parsing it back gives an equal namespace, and
    /// printing that gives the same text.
    #[test]
    fn a_printed_namespace_parses_back_equal(relations in namespace()) {
        let checked = Namespace::new(relations).expect("a valid namespace");
        let text = namespace::print(&t(), &checked).expect("a printable namespace");
        let document = namespace::parse(&text)
            .map_err(|errors| TestCaseError::fail(format!("{text}\n{errors:?}")))?;
        prop_assert_eq!(declared(&document.relations), declared(&checked));
        prop_assert_eq!(namespace::print(&t(), &document.relations), Ok(text));
    }

    /// Expression text built from the grammar's tokens either is refused, or
    /// parses to a tree that prints and parses back equal.
    #[test]
    fn expression_text_parses_print_and_parse_alike(
        tokens in vec(proptest::sample::select(TOKENS.to_vec()), 1..40),
    ) {
        let expression = tokens.concat();
        let json = format!(
            r#"{{ "t": {{ "r": {}, "r0": {{ "constraints": ["identity"] }}, "r1": {{ "constraints": ["identity"] }}, "r2": {{ "constraints": ["identity"] }} }} }}"#,
            value(&expression)
        );
        if let Ok(document) = namespace::parse(&json) {
            let text = namespace::print(&t(), &document.relations)
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            let again = namespace::parse(&text)
                .map_err(|errors| TestCaseError::fail(format!("{text}\n{errors:?}")))?;
            prop_assert_eq!(listed(&again), listed(&document));
            prop_assert_eq!(namespace::print(&t(), &again.relations), Ok(text));
        }
    }

    /// The parser never panics, whatever text it is given.
    #[test]
    fn any_text_is_parsed_or_refused(text in any::<String>()) {
        let _ = namespace::parse(&text);
    }

    /// The parser never panics on near-JSON text, or on any expression.
    #[test]
    fn any_near_json_is_parsed_or_refused(
        text in r#"[{}\[\]":, tThisr0-9_.|&!()\\]{0,200}"#,
        expression in any::<String>(),
    ) {
        let _ = namespace::parse(&text);
        let json = format!(r#"{{ "t": {{ "r": {} }} }}"#, serde_json::to_string(&expression).unwrap());
        let _ = namespace::parse(&json);
    }
}

// Hostile input.

#[test]
fn the_longest_chain_of_relations_is_checked() {
    let chain = namespace_of(MAX_RELATIONS, |i| format!("r{}", i + 1));
    let errors = refused(&chain);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].relation.as_deref(), Some("r499"));
}

#[test]
fn the_largest_set_of_relations_reaching_each_other_is_one_cycle() {
    let count = MAX_RELATIONS;
    let ring = namespace_of(count, |i| {
        format!("r{} | r{}", (i + 1) % count, (i + 7) % count)
    });
    let error = only(&ring);
    let ErrorKind::Namespace(NamespaceError::RewriteCycle { path, .. }) = error.kind else {
        panic!("expected a cycle, got {error}");
    };
    assert_eq!(path.first(), path.last());
    assert_eq!(error.relation.as_deref(), Some("r0"));
}

#[test]
fn far_more_relations_than_the_limit_are_refused() {
    let count = 200_000;
    let many = namespace_of(count, |_| "a".to_owned());
    assert!(many.len() <= MAX_DOCUMENT_BYTES);
    let error = only(&many);
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
    let raw = format!(
        r#"{{ "t": {{ "a": {{ "rewrite": "this{}", "constraints": ["identity"] }} }} }}"#,
        char::from(0)
    );
    assert!(matches!(only(&raw).kind, ErrorKind::Json(_)));
    let escaped = format!(
        r#"{{ "t": {{ "a": {{ "rewrite": "this{}u0000", "constraints": ["identity"] }} }} }}"#,
        '\\'
    );
    assert!(matches!(only(&escaped).kind, ErrorKind::Escape(_)));
}

#[test]
fn a_byte_order_mark_is_refused() {
    let bom = char::from_u32(0xFEFF).unwrap();
    let error = only(&format!(
        r#"{bom}{{ "t": {{ "a": {{ "constraints": ["identity"] }} }} }}"#
    ));
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
    assert!(matches!(nested(r#"{"a":"#, "}"), ErrorKind::UnknownKey(_)));
    assert!(matches!(
        nested(r#"{"rewrite":"#, "}"),
        ErrorKind::NotAnExpression
    ));
    assert!(matches!(
        nested(r#"{"constraints":["#, "]}"),
        ErrorKind::NotConstraints
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
    let error = only(&format!(
        r#"{{ "t": {{ "{long}": {{ "constraints": ["identity"] }} }} }}"#
    ));
    let quoted = format!("{}...", "n".repeat(MAX_QUOTE_BYTES - 3));
    assert_eq!(error.relation.as_deref(), Some(quoted.as_str()));
    assert!(matches!(&error.kind, ErrorKind::Name(NameError::TooLong(name, _)) if *name == quoted));
    // A cut never splits a character: 20 three-byte characters fill 60
    // bytes, and a 21st leaves no room for the cut mark.
    let wide = "€".repeat(30);
    let error = only(&format!(
        r#"{{ "t": {{ "{wide}": {{ "constraints": ["identity"] }} }} }}"#
    ));
    let ErrorKind::Name(NameError::TooLong(name, _)) = &error.kind else {
        panic!("expected a name too long, got {error}");
    };
    assert_eq!(*name, format!("{}...", "€".repeat(20)));
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
            only(&format!(
                r#"{{ "t": {{ "a{c}b": {{ "constraints": ["identity"] }} }} }}"#
            )),
            only(&format!(r#"{{ "t": {{ "a": "a | {c}" }} }}"#)),
            only(&format!(
                r#"{{ "t{c}": {{ "a": {{ "constraints": ["identity"] }} }} }}"#
            )),
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
    let error = only(&format!(
        r#"{{ "t": {{ "a{backslash}u202eb": {{ "constraints": ["identity"] }} }} }}"#
    ));
    assert!(matches!(&error.kind, ErrorKind::Escape(_)));
    let message = error.to_string();
    assert!(
        !message.contains(char::from_u32(0x202e).unwrap()),
        "{message}"
    );
    assert!(message.contains("u{202e}"), "{message}");
}

// The grammar, token by token.

use grammar::DECLARED;

/// A document whose relation `r` has `expression`, beside every relation in
/// [`DECLARED`].
fn grammar_case(expression: &str) -> String {
    let declared: Vec<_> = DECLARED
        .iter()
        .map(|name| format!(r#""{name}": {{ "constraints": ["identity"] }}"#))
        .collect();
    format!(
        r#"{{ "t": {{ "r": {}, {} }} }}"#,
        value(expression),
        declared.join(", ")
    )
}

/// A union of at least one operand.
fn union(operands: Vec<Rewrite<RelationName>>) -> Rewrite<RelationName> {
    Union(NonEmpty::new(operands).expect("at least one operand"))
}

/// An intersection of at least one operand.
fn intersection(operands: Vec<Rewrite<RelationName>>) -> Rewrite<RelationName> {
    Intersection(NonEmpty::new(operands).expect("at least one operand"))
}

#[test]
fn every_valid_form_parses_to_its_tree_and_prints_canonically() {
    for (expression, tree, canonical) in grammar::valid() {
        let document = parsed(&grammar_case(expression));
        assert_eq!(listed(&document)[0].1, tree, "{expression}");
        assert_eq!(printed_expression(tree.clone()), canonical, "{expression}");
        let again = parsed(&grammar_case(canonical));
        assert_eq!(listed(&again)[0].1, tree, "{expression} as {canonical}");
    }
}

#[test]
fn every_invalid_form_is_refused_where_it_goes_wrong() {
    for (expression, column, found, expected) in grammar::invalid() {
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
    for (expression, column) in grammar::reserved() {
        let expression = expression.as_str();
        let error = only(&grammar_case(expression));
        assert_eq!(error.column, Some(column), "{expression}");
        assert!(
            matches!(
                error.kind,
                ErrorKind::Name(NameError::Reserved(_) | NameError::TooLong(..))
            ),
            "{expression}: {error}"
        );
    }
}

#[test]
fn the_relation_limit_counts_distinct_relations() {
    let mut document = namespace_of(MAX_RELATIONS, |_| "this".to_owned());
    document.insert_str(
        document.len() - 4,
        r#", "r0": { "constraints": ["identity"] }"#,
    );
    let error = only(&document);
    assert_eq!(error.relation.as_deref(), Some("r0"));
    assert!(matches!(
        error.kind,
        ErrorKind::Namespace(NamespaceError::DuplicateRelation(_))
    ));
}

// Constraints.

#[test]
fn the_constraints_form_parses_with_and_without_a_rewrite() {
    let document = parsed(
        r#"{ "t": {
        "owner": { "constraints": ["identity"] },
        "editor": { "rewrite": "this | owner", "constraints": ["group#member", "drive.file", "t#owner"] },
        "viewer": { "rewrite": "editor" }
    } }"#,
    );
    let declared = declared(&document.relations);
    assert_eq!(
        declared,
        [
            (relation("owner"), This, vec![written("identity")]),
            (
                relation("editor"),
                Union(nonempty![This, name("owner")]),
                vec![
                    written("drive.file"),
                    written("group#member"),
                    written("t#owner")
                ],
            ),
            (relation("viewer"), name("editor"), vec![]),
        ]
    );
}

#[test]
fn a_malformed_constraints_form_is_one_error_at_its_relation() {
    let backslash = '\\';
    for (value, expected) in [
        (r#"{}"#.to_owned(), "NotAnExpression"),
        (r#"{ "rewrite": "" }"#.to_owned(), "NotAnExpression"),
        (
            r#"{ "rewrite": 1, "constraints": ["identity"] }"#.to_owned(),
            "NotAnExpression",
        ),
        (
            r#"{ "rewrite": ["this"], "constraints": ["identity"] }"#.to_owned(),
            "NotAnExpression",
        ),
        (
            r#"{ "rewrite": { "a": 1 }, "constraints": ["identity"] }"#.to_owned(),
            "NotAnExpression",
        ),
        (r#"{ "constraints": [] }"#.to_owned(), "NotConstraints"),
        (
            r#"{ "constraints": "identity" }"#.to_owned(),
            "NotConstraints",
        ),
        (
            r#"{ "constraints": ["identity", 1] }"#.to_owned(),
            "NotConstraints",
        ),
        (r#"{ "constraints": [null] }"#.to_owned(), "NotConstraints"),
        (
            r#"{ "constraints": { "a": 1 } }"#.to_owned(),
            "NotConstraints",
        ),
        (r#"{ "constraints": true }"#.to_owned(), "NotConstraints"),
        (r#"{ "constraints": 1 }"#.to_owned(), "NotConstraints"),
        (r#"{ "constraints": -1 }"#.to_owned(), "NotConstraints"),
        (r#"{ "constraints": 1.5 }"#.to_owned(), "NotConstraints"),
        (r#"{ "constraints": null }"#.to_owned(), "NotConstraints"),
        (
            r#"{ "constraints": [["identity"]] }"#.to_owned(),
            "NotConstraints",
        ),
        (
            r#"{ "constraints": ["identity"], "subjects": ["identity"] }"#.to_owned(),
            "UnknownKey",
        ),
        (
            r#"{ "constraints": ["identity"], "constraints": ["identity"] }"#.to_owned(),
            "DuplicateKey",
        ),
        (
            r#"{ "rewrite": "this", "rewrite": "this", "constraints": ["identity"] }"#.to_owned(),
            "DuplicateKey",
        ),
        (
            format!(r#"{{ "constraints": ["identity"], "rewrit{backslash}u0065": "this" }}"#),
            "Escape",
        ),
        (
            format!(r#"{{ "constraints": ["identit{backslash}u0079"] }}"#),
            "Escape",
        ),
        (
            format!(r#"{{ "rewrite": "thi{backslash}u0073", "constraints": ["identity"] }}"#),
            "Escape",
        ),
        (r#"{ "constraints": ["a#"] }"#.to_owned(), "Name"),
        (r##"{ "constraints": ["#a"] }"##.to_owned(), "Name"),
        (r#"{ "constraints": ["a b"] }"#.to_owned(), "Name"),
        (r#"{ "constraints": ["a#b#c"] }"#.to_owned(), "Name"),
        (r#"{ "constraints": ["a.#b"] }"#.to_owned(), "Name"),
        (r#"{ "constraints": ["t#this"] }"#.to_owned(), "Name"),
        (r#"{ "constraints": ["t#namespace"] }"#.to_owned(), "Name"),
        (
            r#"{ "constraints": ["identity#member"] }"#.to_owned(),
            "Name",
        ),
        (r#"{ "constraints": [""] }"#.to_owned(), "Name"),
        (
            r#"{ "rewrite": "this |", "constraints": ["identity"] }"#.to_owned(),
            "Syntax",
        ),
        (r#"{ "rewrite": "this |" }"#.to_owned(), "Syntax"),
    ] {
        let json = format!(r#"{{ "t": {{ "r": {value} }} }}"#);
        let errors = refused(&json);
        assert_eq!(errors.len(), 1, "{value}: {errors:?}");
        assert_eq!(errors[0].relation.as_deref(), Some("r"), "{value}");
        let kind = format!("{:?}", errors[0].kind);
        assert!(kind.starts_with(expected), "{value}: {kind}");
    }
}

#[test]
fn a_constraints_form_error_names_what_it_expects() {
    let error = only(r#"{ "t": { "r": { "subjects": ["identity"] } } }"#);
    assert_eq!(
        error.to_string(),
        "relation 'r': 'subjects' isn't a key of a relation's object, which has 'constraints' and 'rewrite'"
    );
    let error = only(r#"{ "t": { "r": { "constraints": [] } } }"#);
    assert_eq!(
        error.to_string(),
        "relation 'r': 'constraints' is a non-empty array of strings"
    );
    let error = only(r#"{ "t": { "r": { "rewrite": "this", "rewrite": "this" } } }"#);
    assert_eq!(
        error.to_string(),
        "relation 'r': 'rewrite' is written twice in a relation's object"
    );
}

#[test]
fn a_relation_whose_constraints_fail_reports_nothing_more() {
    // `r` reads `this`, and its one constraint fails: it's still declared,
    // and isn't reported again as having no constraints.
    let errors = refused(r#"{ "t": { "r": { "constraints": ["a b"] }, "s": "r" } }"#);
    assert_eq!(errors.len(), 1, "{errors:?}");
    // A good constraint beside a failed one is kept.
    let errors = refused(
        r#"{ "t": { "r": { "rewrite": "owner", "constraints": ["a b", "identity"] }, "owner": { "constraints": ["identity"] } } }"#,
    );
    let kinds: Vec<_> = errors
        .iter()
        .map(|error| format!("{:?}", error.kind))
        .collect();
    assert_eq!(kinds.len(), 2, "{kinds:?}");
    assert!(
        kinds[1].starts_with("Namespace(ConstrainedComputed"),
        "{kinds:?}"
    );
}

#[test]
fn a_relation_takes_at_most_32_constraints() {
    let document = |count: usize| {
        let constraints: Vec<_> = (0..count).map(|i| format!(r#""n{i}""#)).collect();
        format!(
            r#"{{ "t": {{ "r": {{ "constraints": [{}] }} }} }}"#,
            constraints.join(", ")
        )
    };
    assert_eq!(declared(&parsed(&document(32)).relations)[0].2.len(), 32);
    assert!(matches!(
        only(&document(33)).kind,
        ErrorKind::Namespace(NamespaceError::TooManyConstraints { count: 33, .. })
    ));
}

#[test]
fn the_printer_leaves_out_this_and_sorts_constraints() {
    let relations = vec![
        (
            "a",
            This,
            vec![
                written("group#member"),
                written("identity"),
                written("drive.file"),
            ],
        ),
        (
            "b",
            Union(nonempty![This, Computed("a")]),
            vec![written("identity")],
        ),
        ("c", Computed("b"), vec![]),
    ];
    assert_eq!(
        namespace::print(&t(), &checked_with(relations)),
        Ok(r#"{
  "t": {
    "a": {
      "constraints": ["drive.file", "group#member", "identity"]
    },
    "b": {
      "rewrite": "this | a",
      "constraints": ["identity"]
    },
    "c": "b"
  }
}
"#
        .to_owned())
    );
}

#[test]
fn a_constraint_error_quotes_its_text_bounded_and_escaped() {
    let long = "x".repeat(300);
    let error = only(&format!(
        r#"{{ "t": {{ "r": {{ "constraints": ["{long} a"] }} }} }}"#
    ));
    let quoted = error.to_string();
    assert!(quoted.len() < 200, "{quoted}");
    let hidden = only("{ \"t\": { \"r\": { \"constraints\": [\"a\u{202e}b\"] } } }");
    assert!(hidden.to_string().contains("\\u{202e}"), "{hidden}");
}

#[test]
fn namespace_and_theory_are_refused_as_relation_names_and_kept_as_namespace_names() {
    for name in ["namespace", "theory", "Namespace", "THEORY"] {
        let error = only(&format!(
            r#"{{ "t": {{ "{name}": {{ "constraints": ["identity"] }} }} }}"#
        ));
        assert!(
            matches!(error.kind, ErrorKind::Name(NameError::Reserved(_))),
            "{name}"
        );
        let error = only(&format!(
            r#"{{ "t": {{ "a": {{ "constraints": ["identity"] }}, "r": "a | {name}" }} }}"#
        ));
        assert!(
            matches!(error.kind, ErrorKind::Name(NameError::Reserved(_))),
            "{name}"
        );
    }
    let document =
        parsed(r#"{ "namespace": { "r": { "constraints": ["theory", "namespace#r"] } } }"#);
    assert_eq!(document.namespace.as_str(), "namespace");
}
