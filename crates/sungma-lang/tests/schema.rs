//! Sungma schema language: parsed, checked whole, and printed back. The
//! grammar cases the namespace document tests hold are the oracle: every
//! form reads each of them the same way.

mod grammar;

use proptest::{collection::vec, prelude::*};
use sungma::{
    name::{NameError, NamespaceName, RelationName},
    namespace::{MAX_REWRITE_DEPTH, Namespace, NamespaceError},
    nonempty,
    rewrite::Rewrite::{self, Computed, This, Union},
};
use sungma_lang::{
    namespace::{
        self, ErrorKind, MAX_DOCUMENT_BYTES, MAX_ERRORS, MAX_EXPRESSION_BYTES, MAX_NAMESPACE_BYTES,
        MAX_RELATIONS, NamespaceDocument, Place, PrintError,
    },
    schema::{self, SchemaError},
};

use grammar::DECLARED;

fn parsed(text: &str) -> Vec<NamespaceDocument> {
    schema::parse(text).unwrap_or_else(|errors| panic!("refused {text:?}: {errors:?}"))
}

fn refused(text: &str) -> Vec<SchemaError> {
    match schema::parse(text) {
        Err(errors) => errors,
        Ok(documents) => panic!("accepted {text:?}: {documents:?}"),
    }
}

/// The one error a document is refused with.
fn only(text: &str) -> SchemaError {
    let mut errors = refused(text);
    assert_eq!(errors.len(), 1, "{text:?}: {errors:?}");
    errors.remove(0)
}

/// The one namespace a document declares.
fn one(text: &str) -> NamespaceDocument {
    let mut documents = parsed(text);
    assert_eq!(documents.len(), 1, "{text:?}");
    documents.remove(0)
}

fn relation(name: &str) -> RelationName {
    RelationName::new_unchecked(name)
}

fn namespace_name(name: &str) -> NamespaceName {
    NamespaceName::new_unchecked(name)
}

/// A namespace's relations, sorted by name, so namespaces from any form
/// compare by what they declare.
fn sorted(relations: &Namespace<RelationName>) -> Vec<(RelationName, Rewrite<RelationName>)> {
    let mut relations: Vec<_> = relations
        .relations()
        .map(|(name, rewrite)| (name.clone(), rewrite.clone()))
        .collect();
    relations.sort_by(|(one, _), (other, _)| one.cmp(other));
    relations
}

/// The rewrite of relation `name` in a namespace.
fn rewrite_of(document: &NamespaceDocument, name: &str) -> Rewrite<RelationName> {
    document
        .relations
        .relations()
        .find(|(relation, _)| relation.as_str() == name)
        .map(|(_, rewrite)| rewrite.clone())
        .expect("the relation is declared")
}

fn print(theory: Option<&str>, documents: &[NamespaceDocument]) -> Result<String, PrintError> {
    let theory = theory.map(namespace_name);
    let namespaces: Vec<_> = documents
        .iter()
        .map(|document| (&document.namespace, &document.relations))
        .collect();
    schema::print(theory.as_ref(), &namespaces)
}

/// Where `  r = ` leaves an expression on its line.
const INDENT: usize = "  r = ".len();
/// The line relation `r` is on in a [`grammar_case`].
const LINE: usize = DECLARED.len() + 2;

/// A document whose namespace `t` declares every relation in [`DECLARED`]
/// as `this`, then relation `r` with `expression`, last and on line
/// [`LINE`], with nothing after it.
fn grammar_case(expression: &str) -> String {
    let declared: String = DECLARED
        .iter()
        .map(|name| format!("  {name} = this\n"))
        .collect();
    format!("namespace: t\n{declared}  r = {expression}")
}

// The oracle: the grammar cases every form shares.

#[test]
fn every_valid_form_parses_to_its_tree_and_prints_canonically() {
    for (expression, tree, canonical) in grammar::valid() {
        let document = one(&grammar_case(expression));
        assert_eq!(rewrite_of(&document, "r"), tree, "{expression}");
        let text = print(None, &[document]).expect("a printable namespace");
        let line = text
            .lines()
            .find(|line| line.starts_with("  r = "))
            .expect("relation r is printed");
        assert_eq!(&line[INDENT..], canonical, "{expression}");
        let again = one(&grammar_case(canonical));
        assert_eq!(rewrite_of(&again, "r"), tree, "{expression} as {canonical}");
    }
}

#[test]
fn every_invalid_form_is_refused_where_it_goes_wrong() {
    for (expression, column, found, expected) in grammar::invalid() {
        // A name then `:` is the next relation's signature here, so the
        // rule ends before it, and is missing.
        let (column, found, expected) = match expression {
            "a:b" => (1, "the end", grammar::TERM),
            _ => (column, found, expected),
        };
        // `a:b` is two mistakes here, the missing rewrite reported first.
        let error = match expression {
            "a:b" => refused(&grammar_case(expression)).remove(0),
            _ => only(&grammar_case(expression)),
        };
        assert_eq!(error.namespace.as_deref(), Some("t"), "{expression}");
        assert_eq!(error.relation.as_deref(), Some("r"), "{expression}");
        let place = Place {
            line: LINE,
            column: column + INDENT,
        };
        assert_eq!(error.place, Some(place), "{expression}");
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
        let error = only(&grammar_case(&expression));
        let place = Place {
            line: LINE,
            column: column + INDENT,
        };
        assert_eq!(error.place, Some(place), "{expression}");
        assert!(
            matches!(
                error.kind,
                ErrorKind::Name(NameError::Reserved(_) | NameError::TooLong(..))
            ),
            "{expression}: {error}"
        );
    }
}

// Documents.

/// The `file` and `folder` namespaces of the namespace document fixtures.
const DRIVE: &str = "theory: drive

// Files sit in folders, and a file's viewers include its folder's.
namespace: file
  owner = this
  parent = this
  editor = this | owner
  viewer = (this | editor | (parent, viewer)) ! banned
  auditor = this & viewer
  banned = this

namespace: folder
  owner = this
  parent = this
  viewer = (this | (parent, viewer)) ! banned
  banned = this
";

fn fixture(name: &str) -> NamespaceDocument {
    let json = match name {
        "file" => include_str!("../../sungma/tests/fixtures/namespaces/file.json"),
        _ => include_str!("../../sungma/tests/fixtures/namespaces/folder.json"),
    };
    namespace::parse(json).expect("a fixture parses")
}

#[test]
fn a_theory_parses_to_its_namespaces_under_their_full_names() {
    let documents = parsed(DRIVE);
    let names: Vec<_> = documents.iter().map(|d| d.namespace.as_str()).collect();
    assert_eq!(names, ["drive.file", "drive.folder"]);
    for (document, fixture) in documents.iter().zip([fixture("file"), fixture("folder")]) {
        assert_eq!(sorted(&document.relations), sorted(&fixture.relations));
    }
}

#[test]
fn without_a_theory_names_are_full() {
    let documents = parsed("namespace: drive.file\n  owner = this\nnamespace: folder\n");
    let names: Vec<_> = documents.iter().map(|d| d.namespace.as_str()).collect();
    assert_eq!(names, ["drive.file", "folder"]);
}

#[test]
fn one_namespace_without_a_theory_is_a_namespace_document() {
    let file = fixture("file");
    let text = print(None, std::slice::from_ref(&file)).expect("a printable namespace");
    let document = one(&text);
    assert_eq!(document.namespace, file.namespace);
    assert_eq!(sorted(&document.relations), sorted(&file.relations));
}

#[test]
fn the_theory_prefixes_every_header() {
    let documents = parsed("theory: a.b\nnamespace: c.d\nnamespace: e\n");
    let names: Vec<_> = documents.iter().map(|d| d.namespace.as_str()).collect();
    assert_eq!(names, ["a.b.c.d", "a.b.e"]);
}

#[test]
fn a_namespace_without_relations_is_empty() {
    let document = one("namespace: user");
    assert_eq!(document.relations.relations().count(), 0);
}

#[test]
fn a_rewrite_can_start_on_the_next_line_and_wrap() {
    let one_line = one("namespace: t\n  a = this\n  r = this | a ! a\n");
    let cases = [
        "namespace: t\n  a = this\n  r\n    = this | a ! a\n",
        "namespace: t\n  a = this\n  r =\n    this\n    | a\n    ! a\n",
        "namespace: t a = this r = this | a ! a",
        "namespace:t\r\n\ta=this\r\n\tr=this|a!a",
    ];
    for text in cases {
        assert_eq!(
            sorted(&one(text).relations),
            sorted(&one_line.relations),
            "{text:?}"
        );
    }
}

#[test]
fn comments_change_nothing() {
    let plain = parsed(DRIVE);
    let commented = DRIVE
        .replace(
            "\nnamespace: folder",
            "// folders\nnamespace: folder // and more\n",
        )
        .replace("banned = this\n", "banned = this // é \u{202e} //\n")
        .replace(
            "editor = this | owner",
            "editor = this // first\n    | owner",
        );
    let commented = parsed(&format!("// top\n{commented}// last"));
    for (one, other) in plain.iter().zip(&commented) {
        assert_eq!(one.namespace, other.namespace);
        assert_eq!(sorted(&one.relations), sorted(&other.relations));
    }
}

#[test]
fn relations_named_like_keywords_are_relations() {
    let document = one("namespace: t\n  namespace = this\n  theory = namespace\n");
    assert_eq!(
        rewrite_of(&document, "theory"),
        Computed(relation("namespace"))
    );
}

#[test]
fn accepted_subjects_are_refused_until_the_model_holds_them() {
    let error = only("namespace: t\n  owner: identity\n");
    assert_eq!(error.relation.as_deref(), Some("owner"));
    assert_eq!(error.place, Some(Place { line: 2, column: 8 }));
    assert_eq!(error.kind.to_string(), "expected '=', found ':'");
}

#[test]
fn what_isnt_in_the_grammar_is_refused() {
    let cases = [
        ("namespace: t\n  r = \"this\"", 2, 7, "'\"'"),
        ("namespace: t\n  r = 'this'", 2, 7, "'''"),
        ("namespace: t\n  r = 1", 2, 7, "'1'"),
        ("namespace: t\n  r = this # comment", 2, 12, "'#'"),
        ("namespace: t\n  r = this / comment", 2, 12, "'/'"),
        ("namespace: t\n  r = é", 2, 7, "'é'"),
        ("namespace: t\n  r = \0", 2, 7, "'\\u{0000}'"),
    ];
    for (text, line, column, found) in cases {
        let error = only(text);
        assert_eq!(error.place, Some(Place { line, column }), "{text:?}");
        assert!(
            error.kind.to_string().ends_with(&format!("found {found}")),
            "{text:?}: {error}"
        );
    }
}

#[test]
fn a_byte_order_mark_is_refused() {
    let error = only("\u{feff}namespace: t\n");
    assert!(matches!(error.kind, ErrorKind::ByteOrderMark), "{error}");
    assert_eq!(error.place, None);
}

#[test]
fn a_document_declares_at_least_one_namespace() {
    for text in ["", "  \n", "// only a comment", "theory: t\n"] {
        let error = only(text);
        assert!(matches!(error.kind, ErrorKind::NoNamespace), "{text:?}");
    }
}

#[test]
fn relations_belong_to_a_namespace() {
    let error = only("theory: t\n  r = this\nnamespace: n\n");
    assert_eq!(error.namespace, None);
    assert_eq!(error.place, Some(Place { line: 2, column: 3 }));
    assert_eq!(
        error.kind.to_string(),
        "expected 'namespace:' or the end, found 'r'"
    );
}

#[test]
fn the_theory_comes_first() {
    let error = only("namespace: n\n  r = this\ntheory: t\n");
    assert_eq!(error.relation.as_deref(), Some("theory"));
    assert_eq!(error.kind.to_string(), "expected '=', found ':'");
}

#[test]
fn a_namespace_name_is_written_without_spaces() {
    let cases = [
        (
            "namespace: a .b",
            1,
            14,
            "expected a relation, 'namespace:' or the end, found '.'",
        ),
        ("namespace: a. b", 1, 12, "is not a valid name"),
        ("namespace: .a", 1, 12, "is not a valid name"),
        ("namespace: a..b", 1, 12, "is not a valid name"),
        ("namespace: 1a", 1, 12, "'1a' is not a valid name"),
        (
            "namespace:",
            1,
            11,
            "expected a namespace name, found the end",
        ),
        (
            "theory: a b",
            1,
            11,
            "expected 'namespace:' or the end, found 'b'",
        ),
    ];
    for (text, line, column, message) in cases {
        let error = refused(text).remove(0);
        assert_eq!(
            error.place,
            Some(Place { line, column }),
            "{text:?}: {error}"
        );
        assert!(error.to_string().contains(message), "{text:?}: {error}");
    }
}

#[test]
fn a_namespace_declared_twice_is_refused() {
    let cases = [
        "namespace: a\nnamespace: b\nnamespace: a\n",
        "theory: t\nnamespace: a\nnamespace: a\n",
    ];
    for text in cases {
        let error = only(text);
        assert_eq!(error.place.map(|place| place.line), Some(3), "{text:?}");
        assert!(
            matches!(error.kind, ErrorKind::DuplicateNamespace(_)),
            "{text:?}: {error}"
        );
    }
}

#[test]
fn one_bad_namespace_refuses_the_document() {
    let text = format!("{DRIVE}\nnamespace: other\n  r = undeclared\n");
    let error = only(&text);
    assert_eq!(error.namespace.as_deref(), Some("drive.other"));
    assert_eq!(error.relation.as_deref(), Some("r"));
    assert_eq!(
        error.place,
        Some(Place {
            line: 19,
            column: 3
        })
    );
    assert!(
        matches!(
            error.kind,
            ErrorKind::Namespace(NamespaceError::DanglingReference { .. })
        ),
        "{error}"
    );
}

#[test]
fn namespace_checks_apply_to_each_namespace() {
    let errors = refused("namespace: a\n  r = s\n  s = r\nnamespace: b\n  r = this\n  r = this\n");
    let kinds: Vec<_> = errors.iter().map(|error| error.kind.to_string()).collect();
    assert_eq!(errors.len(), 2, "{kinds:?}");
    assert_eq!(errors[0].namespace.as_deref(), Some("a"));
    assert!(matches!(
        errors[0].kind,
        ErrorKind::Namespace(NamespaceError::RewriteCycle { .. })
    ));
    assert_eq!(errors[1].namespace.as_deref(), Some("b"));
    assert_eq!(errors[1].place, Some(Place { line: 6, column: 3 }));
    assert!(matches!(
        errors[1].kind,
        ErrorKind::Namespace(NamespaceError::DuplicateRelation(_))
    ));
}

// Errors.

#[test]
fn every_problem_is_reported_in_document_order_once() {
    let text = "namespace: t
  a = this |
  b = this
  c = this )
  d = undeclared
  e = (this
  f = this
";
    let errors = refused(text);
    let places: Vec<_> = errors
        .iter()
        .map(|error| (error.relation.clone(), error.place.map(|place| place.line)))
        .collect();
    assert_eq!(
        places,
        [
            (Some("a".to_owned()), Some(3)),
            (Some("c".to_owned()), Some(4)),
            (Some("d".to_owned()), Some(5)),
            (Some("e".to_owned()), Some(7)),
        ],
        "{errors:?}"
    );
}

#[test]
fn a_relation_whose_rewrite_fails_is_still_declared() {
    let error = only("namespace: t\n  a = this (\n  b = a\n");
    assert_eq!(error.relation.as_deref(), Some("a"));
}

#[test]
fn errors_stop_at_the_cap() {
    let relations: String = (0..MAX_ERRORS + 5)
        .map(|i| format!("  r{i} = undeclared\n"))
        .collect();
    let errors = refused(&format!("namespace: t\n{relations}"));
    assert_eq!(errors.len(), MAX_ERRORS + 1);
    assert!(matches!(errors[MAX_ERRORS].kind, ErrorKind::TooManyErrors));
    assert_eq!(errors[MAX_ERRORS - 1].relation.as_deref(), Some("r49"));
}

#[test]
fn errors_at_the_cap_are_all_reported() {
    let relations: String = (0..MAX_ERRORS)
        .map(|i| format!("  r{i} = undeclared\n"))
        .collect();
    let errors = refused(&format!("namespace: t\n{relations}"));
    assert_eq!(errors.len(), MAX_ERRORS);
    assert!(
        errors
            .iter()
            .all(|error| !matches!(error.kind, ErrorKind::TooManyErrors))
    );
}

#[test]
fn errors_name_their_place() {
    let error = only("namespace: t\n  // é\n  r = é");
    assert_eq!(
        error.to_string(),
        "line 3, column 7: namespace 't', relation 'r': \
         expected 'this', a relation name or '(', found 'é'"
    );
    let error = only("namespace: t\n  r = this\n  r = this");
    assert_eq!(
        error.to_string(),
        "line 3, column 3: namespace 't', relation 'r': relation 'r' is declared more than once"
    );
    let error = only("theory: t\nnamespace: a\nnamespace: a");
    assert_eq!(
        error.to_string(),
        "line 3, column 1: namespace 't.a': namespace 't.a' is declared twice"
    );
    let error = only("\u{feff}");
    assert_eq!(
        error.to_string(),
        "a document doesn't start with a byte-order mark"
    );
}

#[test]
fn columns_count_characters_on_their_line() {
    let error = only("namespace: t\n  a = this // ü ü\n  r = this | ü");
    assert_eq!(
        error.place,
        Some(Place {
            line: 3,
            column: 14
        })
    );
    let error = only("namespace: t\n  r = this\n    | this\n    ! ü");
    assert_eq!(error.place, Some(Place { line: 4, column: 7 }));
}

#[test]
fn an_error_quotes_a_bounded_and_escaped_cut() {
    let long = "n".repeat(300);
    let error = only(&format!("namespace: t\n  r = {long}"));
    assert!(error.to_string().len() < 200, "{error}");
    let error = only("namespace: t\n  r = \u{202e}");
    assert!(error.to_string().contains("u{202e}"), "{error}");
}

// Limits.

#[test]
fn each_limit_holds_at_its_value_and_breaks_one_past_it() {
    let relations = |count: usize| -> String {
        let relations: String = (0..count).map(|i| format!("  r{i} = this\n")).collect();
        format!("namespace: t\n{relations}")
    };
    one(&relations(MAX_RELATIONS));
    let error = only(&relations(MAX_RELATIONS + 1));
    assert!(
        matches!(error.kind, ErrorKind::TooManyRelations(count) if count == MAX_RELATIONS + 1),
        "{error}"
    );
    assert_eq!(error.place, Some(Place { line: 1, column: 1 }));

    // A rule of exactly `bytes`: `this`, runs of ` | this`, and a last
    // relation whose name makes up the rest. Each text declares that
    // relation and the rule.
    let rule = |bytes: usize| {
        let runs = (bytes - 4 - 3 - 1) / 7;
        let last = "n".repeat(bytes - 4 - 3 - 7 * runs);
        let rule = format!("this{} | {last}", " | this".repeat(runs));
        assert_eq!(rule.len(), bytes);
        (format!("namespace: t\n  {last} = this\n  r = "), rule)
    };
    for bytes in [MAX_EXPRESSION_BYTES - 1, MAX_EXPRESSION_BYTES] {
        let (head, rule) = rule(bytes);
        one(&format!("{head}{rule}\n  // a comment after the rule\n"));
    }
    // The rule is measured from its first token to its last, so a comment
    // inside it counts.
    let (head, full) = rule(MAX_EXPRESSION_BYTES);
    let commented = full.replacen(" | ", " // c\n | ", 1);
    assert!(matches!(
        only(&format!("{head}{commented}")).kind,
        ErrorKind::ExpressionTooLong
    ));
    let (head, rule) = rule(MAX_EXPRESSION_BYTES + 1);
    let error = only(&format!("{head}{rule}"));
    assert!(
        matches!(error.kind, ErrorKind::ExpressionTooLong),
        "{error}"
    );
    assert_eq!(error.place, Some(Place { line: 3, column: 7 }));

    let name = "n".repeat(MAX_NAMESPACE_BYTES - 2);
    one(&format!("theory: t\nnamespace: {name}"));
    let error = only(&format!("theory: tt\nnamespace: {name}"));
    assert!(
        matches!(error.kind, ErrorKind::Name(NameError::TooLong(..))),
        "{error}"
    );

    let filler = "// .\n".repeat(MAX_DOCUMENT_BYTES / 5);
    let text = format!("{filler}namespace: t\n");
    assert!(text.len() > MAX_DOCUMENT_BYTES);
    assert!(matches!(only(&text).kind, ErrorKind::TooLarge));
    let text = format!("{}namespace: t", " ".repeat(MAX_DOCUMENT_BYTES - 12));
    assert_eq!(text.len(), MAX_DOCUMENT_BYTES);
    one(&text);
}

#[test]
fn parentheses_nest_at_most_max_depth() {
    let nested = |depth: usize| format!("{}this{}", "(".repeat(depth), ")".repeat(depth));
    one(&format!(
        "namespace: t\n  r = {}",
        nested(MAX_REWRITE_DEPTH)
    ));
    let error = only(&format!(
        "namespace: t\n  r = {}",
        nested(MAX_REWRITE_DEPTH + 1)
    ));
    assert!(
        matches!(error.kind, ErrorKind::TooManyParentheses),
        "{error}"
    );
    let column = INDENT + MAX_REWRITE_DEPTH + 1;
    assert_eq!(error.place, Some(Place { line: 2, column }));
}

// Hostile input.

#[test]
fn far_more_relations_than_the_limit_are_refused() {
    let relations: String = (0..20 * MAX_RELATIONS)
        .map(|i| format!("r{i}=r{} ", i + 1))
        .collect();
    let error = only(&format!("namespace: t {relations}"));
    assert!(
        matches!(error.kind, ErrorKind::TooManyRelations(_)),
        "{error}"
    );
}

#[test]
fn the_longest_chain_of_relations_is_checked() {
    let relations: String = (0..MAX_RELATIONS)
        .map(|i| {
            if i + 1 == MAX_RELATIONS {
                format!("  r{i} = this\n")
            } else {
                format!("  r{i} = r{}\n", i + 1)
            }
        })
        .collect();
    one(&format!("namespace: t\n{relations}"));
}

// Printing.

#[test]
fn the_printer_writes_the_canonical_form() {
    let documents = parsed(DRIVE);
    let mut reversed = parsed(DRIVE);
    reversed.reverse();
    let expected = "theory: drive

namespace: file
  auditor = this & viewer
  banned = this
  editor = this | owner
  owner = this
  parent = this
  viewer = (this | editor | (parent, viewer)) ! banned

namespace: folder
  banned = this
  owner = this
  parent = this
  viewer = (this | (parent, viewer)) ! banned
";
    assert_eq!(print(Some("drive"), &documents), Ok(expected.to_owned()));
    assert_eq!(print(Some("drive"), &reversed), Ok(expected.to_owned()));
    let full = print(None, &documents).expect("printable namespaces");
    assert!(full.starts_with("namespace: drive.file\n"), "{full}");
    assert!(full.contains("\n\nnamespace: drive.folder\n"), "{full}");
    let empty = parsed("namespace: user");
    assert_eq!(print(None, &empty), Ok("namespace: user\n".to_owned()));
}

#[test]
fn the_printer_refuses_caller_defects() {
    let documents = parsed(DRIVE);
    assert_eq!(print(None, &[]), Err(PrintError::NoNamespace));
    assert_eq!(
        print(Some("drives"), &documents),
        Err(PrintError::OutsideTheory("drive.file".to_owned()))
    );
    assert_eq!(
        print(Some("driv"), &documents),
        Err(PrintError::OutsideTheory("drive.file".to_owned()))
    );
    assert!(matches!(
        print(Some("bad name"), &documents),
        Err(PrintError::Name(_))
    ));
    let mut twice = parsed(DRIVE);
    twice.extend(parsed(DRIVE));
    assert_eq!(
        print(Some("drive"), &twice),
        Err(PrintError::DuplicateNamespace("drive.file".to_owned()))
    );
    let one_operand = NamespaceDocument {
        namespace: namespace_name("t"),
        relations: Namespace::new(vec![(relation("r"), Union(nonempty![This]))])
            .expect("a valid namespace"),
    };
    assert_eq!(print(None, &[one_operand]), Err(PrintError::OneOperand));
    let bad_namespace = NamespaceDocument {
        namespace: namespace_name("a..b"),
        relations: Namespace::new(vec![]).expect("a valid namespace"),
    };
    assert!(matches!(
        print(None, &[bad_namespace]),
        Err(PrintError::Name(_))
    ));
}

// Round trips, and the oracle on generated expressions.

const TOKENS: [&str; 12] = [
    "this", "r0", "r1", "r2", "(", ")", ",", "|", "&", "!", " ", "\n",
];

proptest! {
    /// Expression text built from the grammar's tokens is accepted by the
    /// schema language exactly when the JSON form accepts it, as the same
    /// tree, and an accepted one prints and parses back equal.
    #[test]
    fn the_schema_language_reads_expressions_as_json_does(
        tokens in vec(proptest::sample::select(TOKENS.to_vec()), 1..40),
    ) {
        let expression = tokens.concat();
        let json = format!(
            r#"{{ "t": {{ "r": "{}", "r0": "this", "r1": "this", "r2": "this" }} }}"#,
            expression.replace('\n', " ")
        );
        let text = format!("namespace: t\n  r0 = this\n  r1 = this\n  r2 = this\n  r = {expression}");
        match (namespace::parse(&json), schema::parse(&text)) {
            (Ok(json), Ok(mut documents)) => {
                let document = documents.remove(0);
                prop_assert_eq!(sorted(&document.relations), sorted(&json.relations));
                let printed = print(None, std::slice::from_ref(&document))
                    .map_err(|error| TestCaseError::fail(error.to_string()))?;
                let again = one(&printed);
                prop_assert_eq!(sorted(&again.relations), sorted(&document.relations));
                prop_assert_eq!(print(None, &[again]), Ok(printed));
            }
            (Err(_), Err(_)) => {}
            (json, schema) => {
                return Err(TestCaseError::fail(format!("{expression:?}: {json:?} {schema:?}")));
            }
        }
    }

    /// The parser never panics, whatever text it is given.
    #[test]
    fn any_text_is_parsed_or_refused(text in any::<String>()) {
        let _ = schema::parse(&text);
    }

    /// The parser never panics on near-schema text.
    #[test]
    fn any_near_schema_text_is_parsed_or_refused(
        text in r"(theory|namespace|this|[:=.,()|&!/ \n#rt0-9])*",
    ) {
        if let Ok(documents) = schema::parse(&text) {
            let printed = print(None, &documents)
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            let again = parsed(&printed);
            prop_assert_eq!(print(None, &again), Ok(printed));
        }
    }
}

// Review findings, each pinned before its fix.

#[test]
fn a_long_chain_of_exclusions_is_refused_without_a_crash() {
    let chain = vec!["b"; 300_000].join("!");
    let error = only(&format!("namespace: t\n  b = this\n  a = {chain}"));
    assert!(
        matches!(error.kind, ErrorKind::ExpressionTooLong),
        "{error}"
    );
}

#[test]
fn the_printer_refuses_a_document_past_the_size_limit() {
    // Each rewrite prints at the expression limit, at twice the bytes it's
    // written in.
    let dense = vec!["b"; 1024].join("|");
    let relations: String = (0..400).map(|i| format!("r{i}={dense}\n")).collect();
    let namespace = |name: &str| format!("namespace: {name}\nb=this\n{relations}");
    let text = format!("{}{}{}", namespace("s"), namespace("t"), namespace("u"));
    assert!(text.len() < MAX_DOCUMENT_BYTES);
    let documents = parsed(&text);
    assert_eq!(print(None, &documents), Err(PrintError::TooLarge));
}

#[test]
fn a_namespace_writes_at_most_max_relations_declarations() {
    let text = format!("namespace: t\n{}", "  a = !\n".repeat(MAX_RELATIONS + 1));
    let error = only(&text);
    assert!(
        matches!(error.kind, ErrorKind::TooManyRelations(count) if count == MAX_RELATIONS + 1),
        "{error}"
    );
}

#[test]
fn the_end_after_a_trailing_comment_is_at_the_end_of_its_line() {
    let error = only("namespace: t\n  r = // note");
    assert_eq!(
        error.place,
        Some(Place {
            line: 2,
            column: 14
        })
    );
}

#[test]
fn a_rewrite_left_open_doesnt_swallow_what_comes_next() {
    for text in [
        "namespace: a\n  r = this |\nnamespace: b\n  r = this",
        "namespace: a\n  r = this |\n  b = this\n  c = b",
        "namespace: a\n  r = this !\n  s = this",
    ] {
        let error = only(text);
        assert_eq!(error.place.map(|place| place.line), Some(3), "{text:?}");
        assert_eq!(error.relation.as_deref(), Some("r"), "{text:?}");
    }
}

#[test]
fn a_bad_theory_name_is_reported_once() {
    let error = only("theory: a..b\nnamespace: c\nnamespace: d\n");
    assert_eq!(error.place, Some(Place { line: 1, column: 9 }));
}

#[test]
fn a_namespace_named_in_full_under_its_theory_is_the_same_namespace() {
    let error = only("theory: t\nnamespace: a\nnamespace: t.a\n");
    assert!(
        matches!(error.kind, ErrorKind::DuplicateNamespace(_)),
        "{error}"
    );
    let documents = parsed("theory: t\nnamespace: t.b\n");
    assert_eq!(documents[0].namespace.as_str(), "t.b");
}

#[test]
fn an_unclosed_parenthesis_doesnt_swallow_the_next_relation() {
    for text in [
        "namespace: x\n  b = this\n  r = (b\n  a = this\n  c = a\n",
        "namespace: x\n  b = this\n  r = (b,\n  a = this\n  c = a\n",
    ] {
        let error = only(text);
        assert_eq!(error.relation.as_deref(), Some("r"), "{text:?}");
    }
}

#[test]
fn namespaces_missing_their_names_arent_duplicates() {
    let errors = refused("namespace: =\nnamespace: =\n");
    assert!(
        errors
            .iter()
            .all(|error| !matches!(error.kind, ErrorKind::DuplicateNamespace(_))),
        "{errors:?}"
    );
}

#[test]
fn a_missing_name_doesnt_take_the_next_lines_keyword() {
    for (text, found) in [
        ("theory:\nnamespace: x\n  a = this\n", "'namespace'"),
        ("namespace:\n  owner = this\n", "'owner'"),
    ] {
        let error = only(text);
        assert_eq!(
            error.kind.to_string(),
            format!("expected a namespace name, found {found}"),
            "{text:?}"
        );
    }
}

#[test]
fn the_printer_refuses_a_rule_that_prints_past_the_limit() {
    let dense = vec!["b"; 2048].join("|");
    let documents = parsed(&format!("namespace: t\n  b = this\n  r = {dense}"));
    assert_eq!(print(None, &documents), Err(PrintError::ExpressionTooLong));
}

#[test]
fn a_malformed_namespace_name_is_one_name_error() {
    for (text, written) in [
        ("namespace: 1abc\n  owner = this\n", "1abc"),
        ("namespace: my-ns\n  r = this\n", "my-ns"),
    ] {
        let error = only(text);
        assert_eq!(
            error.place,
            Some(Place {
                line: 1,
                column: 12
            }),
            "{text:?}"
        );
        assert!(
            matches!(&error.kind, ErrorKind::Name(_) if error.to_string().contains(written)),
            "{text:?}: {error}"
        );
    }
}

#[test]
fn a_document_declares_at_most_max_namespaces() {
    let namespaces =
        |count: usize| -> String { (0..count).map(|i| format!("namespace: n{i}\n")).collect() };
    assert_eq!(
        parsed(&namespaces(schema::MAX_NAMESPACES)).len(),
        schema::MAX_NAMESPACES
    );
    let error = only(&namespaces(schema::MAX_NAMESPACES + 1));
    assert!(
        matches!(error.kind, ErrorKind::TooManyNamespaces(count) if count == schema::MAX_NAMESPACES + 1),
        "{error}"
    );
}
