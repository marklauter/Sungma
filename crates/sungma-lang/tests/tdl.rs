//! TDL documents: parsed, checked whole, and printed back. The grammar
//! cases in `grammar` are the oracle for rewrites.

mod grammar;

use std::time::{Duration, Instant};

use proptest::{collection::vec, prelude::*};
use sungma::{
    name::{NameError, NamespaceName, RelationName},
    namespace::{Constraint, DEFAULT_REWRITE_DEPTH, Namespace, NamespaceError},
    non_empty::NonEmpty,
    nonempty,
    rewrite::Rewrite::{self, Computed, Exclusion, FactTo, Intersection, This, Union},
};
use sungma_lang::tdl::{
    self, DEFAULT_MAX_ERRORS, ErrorKind, MAX_NAMESPACE_BYTES, MAX_QUOTE_BYTES, MAX_RELATION_BYTES,
    Named, NamedConstraint, NamespaceDocument, Place, PrintError, Refusal, Settings, TdlError,
};

use grammar::DECLARED;

/// The size the hostile documents are built to: far past any written
/// document. The parser has no size limit of its own.
const LARGE: usize = 2 * 1024 * 1024;

fn parsed(text: &str) -> Vec<NamespaceDocument> {
    tdl::parse(text).unwrap_or_else(|errors| panic!("refused {text:?}: {errors:?}"))
}

fn refused(text: &str) -> Vec<TdlError> {
    match tdl::parse(text) {
        Err(errors) => errors,
        Ok(documents) => panic!("accepted {text:?}: {documents:?}"),
    }
}

/// The one error a document is refused with.
fn only(text: &str) -> TdlError {
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

fn name(name: &str) -> Rewrite<RelationName> {
    Computed(relation(name))
}

fn fact_to(factset: &str, computed: &str) -> Rewrite<RelationName> {
    FactTo {
        factset: relation(factset),
        computed: relation(computed),
    }
}

fn union(operands: Vec<Rewrite<RelationName>>) -> Rewrite<RelationName> {
    Union(NonEmpty::new(operands).expect("at least one operand"))
}

fn intersection(operands: Vec<Rewrite<RelationName>>) -> Rewrite<RelationName> {
    Intersection(NonEmpty::new(operands).expect("at least one operand"))
}

fn exclusion<N>(base: Rewrite<N>, excluded: Rewrite<N>) -> Rewrite<N> {
    Exclusion(Box::new(base), Box::new(excluded))
}

/// The full names of a document's namespaces, in document order.
fn names(documents: &[NamespaceDocument]) -> Vec<&str> {
    documents
        .iter()
        .map(|document| document.namespace.as_str())
        .collect()
}

/// A namespace's relations, sorted by name, so namespaces compare by what
/// they declare.
fn sorted(relations: &Named) -> Vec<(RelationName, Rewrite<RelationName>)> {
    let mut relations: Vec<_> = relations
        .relations()
        .map(|(name, rewrite)| (name.clone(), rewrite.clone()))
        .collect();
    relations.sort_by(|(one, _), (other, _)| one.cmp(other));
    relations
}

/// A namespace's relations with their constraints, both sorted.
fn declared(relations: &Named) -> Vec<(RelationName, Rewrite<RelationName>, Vec<NamedConstraint>)> {
    let mut declared: Vec<_> = relations
        .relations()
        .map(|(name, rewrite)| {
            let mut constraints = relations.constraints(name).to_vec();
            constraints.sort();
            (name.clone(), rewrite.clone(), constraints)
        })
        .collect();
    declared.sort_by(|(one, ..), (other, ..)| one.cmp(other));
    declared
}

/// The rewrite of relation `name` in a namespace.
fn rewrite_of(document: &NamespaceDocument, name: &str) -> Rewrite<RelationName> {
    document
        .relations
        .rewrite(&relation(name))
        .map(|rewrite| (**rewrite).clone())
        .expect("the relation is declared")
}

/// Each constraint of relation `name`, as a document writes it with full
/// names, sorted.
fn constraints_of(document: &NamespaceDocument, name: &str) -> Vec<String> {
    let mut constraints: Vec<_> = document
        .relations
        .constraints(&relation(name))
        .iter()
        .map(ToString::to_string)
        .collect();
    constraints.sort();
    constraints
}

fn print(documents: &[NamespaceDocument]) -> Result<String, PrintError> {
    let namespaces: Vec<_> = documents
        .iter()
        .map(|document| (&document.namespace, &document.relations))
        .collect();
    tdl::print(&namespaces)
}

/// Whether two parses declare the same namespaces, in any order.
fn same(one: &[NamespaceDocument], other: &[NamespaceDocument]) -> bool {
    let view = |documents: &[NamespaceDocument]| {
        let mut view: Vec<_> = documents
            .iter()
            .map(|document| (document.namespace.clone(), declared(&document.relations)))
            .collect();
        view.sort_by(|(one, _), (other, _)| one.cmp(other));
        view
    };
    view(one) == view(other)
}

fn place(line: usize, column: usize) -> Option<Place> {
    Some(Place { line, column })
}

// The grammar, case by case.

/// The line relation `r` is on in a [`grammar_case`].
const LINE: usize = DECLARED.len() + 2;

/// Whether an expression's text holds the keyword `this`.
fn mentions_this(expression: &str) -> bool {
    expression
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(|word| word == "this")
}

/// Relation `r`'s signature for `expression`: taking identities when it
/// reads `this`.
fn signature(expression: &str) -> &'static str {
    if mentions_this(expression) {
        "  r<@identity>: "
    } else {
        "  r: "
    }
}

/// A document whose namespace `t` declares every relation in [`DECLARED`]
/// taking identities, then relation `r` with `expression`, last and on line
/// [`LINE`], with nothing after it.
fn grammar_case(expression: &str) -> String {
    let declared: String = DECLARED
        .iter()
        .map(|name| format!("  {name}<@identity>\n"))
        .collect();
    format!(
        "namespace: t\n{declared}{}{expression}",
        signature(expression)
    )
}

#[test]
fn every_valid_form_parses_to_its_tree_and_prints_canonically() {
    for (expression, tree, canonical) in grammar::valid() {
        let document = one(&grammar_case(expression));
        assert_eq!(rewrite_of(&document, "r"), tree, "{expression}");
        let text = print(&[document]).expect("a printable namespace");
        let line = text
            .lines()
            .find(|line| line.starts_with("  r:") || line.starts_with("  r<"))
            .expect("relation r is printed");
        let printed = match line {
            "  r<@identity>" => "this",
            _ => &line[signature(canonical).len()..],
        };
        assert_eq!(printed, canonical, "{expression}");
        let again = one(&grammar_case(canonical));
        assert_eq!(rewrite_of(&again, "r"), tree, "{expression} as {canonical}");
    }
}

#[test]
fn every_invalid_form_is_refused_where_it_goes_wrong() {
    for (expression, column, found, expected) in grammar::invalid() {
        let error = only(&grammar_case(expression));
        assert_eq!(error.namespace.as_deref(), Some("t"), "{expression}");
        assert_eq!(error.relation.as_deref(), Some("r"), "{expression}");
        let at = place(LINE, column + signature(expression).len());
        assert_eq!(error.place, at, "{expression}");
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
        let at = place(LINE, column + signature(&expression).len());
        assert_eq!(error.place, at, "{expression}");
        assert!(
            matches!(
                error.kind,
                ErrorKind::Name(NameError::Reserved(_) | NameError::TooLong(..))
            ),
            "{expression}: {error}"
        );
    }
}

/// A name followed by `:` or `<` starts the next relation, so the rewrite
/// before it ends there, empty.
#[test]
fn a_relation_inside_a_rewrite_ends_it() {
    for expression in ["a:b", "a<b"] {
        let errors = refused(&grammar_case(expression));
        assert_eq!(errors[0].relation.as_deref(), Some("r"), "{expression}");
        assert_eq!(
            errors[0].kind.to_string(),
            format!("expected {}, found the end", grammar::TERM),
            "{expression}"
        );
    }
}

// Documents.

/// Sungma's own model, as `docs/specs/theory-documents.md` writes it.
const SAMPLE: &str = "theory: sungma

// The deployment, one resource: sungma.root:sungma
namespace: root
  owner<@identity>
  admin<@identity>: this | owner
  namespace_creator<@identity, group#member>: this | admin
  group_creator<@identity, group#member>: this | admin
  operator<@identity, group#member>: this | admin
  auditor<@identity, group#member>: this | admin
  reader<@identity, group#member>: this | auditor
  writer<@identity, group#member>: this | admin
  fact_writer<@identity, group#member>: this | admin
  fact_reader<@identity, group#member>: this | fact_writer | auditor
  directory_reader<@identity, group#member>: this | auditor
  access_reader: auditor

namespace: namespace
  parent<root>
  protected<root>
  owner<@identity, group#member>: this | (parent, owner) ! (protected, owner)
  admin<@identity, group#member>: this | owner | (parent, admin) ! (protected, admin)
  writer<@identity, group#member>: this | admin | (parent, writer) ! (protected, writer)
  reader<@identity, group#member>: this | writer | fact_reader | auditor | (parent, reader)
  fact_writer<@identity, group#member>: this | admin | (parent, fact_writer) ! (protected, fact_writer)
  fact_reader<@identity, group#member>: this | fact_writer | auditor | (parent, fact_reader)
  auditor<@identity, group#member>: this | (parent, auditor)
  access_reader: admin | auditor

namespace: relation
  parent<namespace>
  writer<@identity, group#member>: this | (parent, fact_writer)
  reader<@identity, group#member>: this | writer | (parent, fact_reader)

namespace: group
  parent<root>
  owner<@identity, group#member>: this | (parent, admin)
  member<@identity, group#member>
  access_reader: owner | (parent, auditor)
";

#[test]
fn the_sample_parses_to_sungmas_four_namespaces() {
    let documents = parsed(SAMPLE);
    assert_eq!(
        names(&documents),
        [
            "sungma.root",
            "sungma.namespace",
            "sungma.relation",
            "sungma.group"
        ]
    );
    assert_eq!(constraints_of(&documents[0], "owner"), ["@identity"]);
    assert_eq!(
        constraints_of(&documents[0], "reader"),
        ["@identity", "sungma.group#member"]
    );
    assert_eq!(rewrite_of(&documents[0], "owner"), This);
    assert_eq!(constraints_of(&documents[0], "access_reader"), [""; 0]);
    assert_eq!(constraints_of(&documents[1], "parent"), ["sungma.root"]);
    assert_eq!(
        constraints_of(&documents[2], "parent"),
        ["sungma.namespace"]
    );
    assert_eq!(
        rewrite_of(&documents[1], "owner"),
        union(vec![
            This,
            exclusion(fact_to("parent", "owner"), fact_to("protected", "owner")),
        ])
    );
    let printed = print(&documents).expect("a printable model");
    assert!(
        printed.starts_with("theory: sungma\n\nnamespace: group\n"),
        "{printed}"
    );
    let again = parsed(&printed);
    assert!(same(&documents, &again));
    assert_eq!(print(&again), Ok(printed));
}

#[test]
fn a_document_declares_at_least_one_namespace() {
    for text in ["", "  \n", "// only a comment", "// one\n// two\n"] {
        let error = only(text);
        assert!(matches!(error.kind, ErrorKind::NoNamespace), "{text:?}");
    }
    let document = one("namespace: user");
    assert_eq!(document.relations.relations().count(), 0);
}

#[test]
fn this_is_implicit_without_a_rewrite_and_a_relation_without_constraints_is_computed() {
    let document = one("namespace: t\n  owner<@identity>\n  viewer: owner\n");
    assert_eq!(rewrite_of(&document, "owner"), This);
    assert_eq!(constraints_of(&document, "owner"), ["@identity"]);
    assert_eq!(rewrite_of(&document, "viewer"), name("owner"));
    assert_eq!(constraints_of(&document, "viewer"), [""; 0]);
    let explicit = one("namespace: t\n  owner<@identity>: this\n  viewer: owner\n");
    assert_eq!(sorted(&explicit.relations), sorted(&document.relations));
    let error = only("namespace: t\n  owner: this\n");
    assert!(
        matches!(
            error.kind,
            ErrorKind::Namespace(NamespaceError::Unconstrained(_))
        ),
        "{error}"
    );
    let error = only("namespace: t\n  a<@identity>\n  owner<@identity>: a\n");
    assert!(
        matches!(
            error.kind,
            ErrorKind::Namespace(NamespaceError::ConstrainedComputed(_))
        ),
        "{error}"
    );
}

#[test]
fn a_relation_needs_constraints_or_a_rewrite() {
    for (text, at, message) in [
        (
            "namespace: t\n  owner\n",
            place(2, 3),
            "expected a relation, 'namespace:', 'theory:' or the end, found 'owner'",
        ),
        (
            "namespace: t\n  owner:\n",
            place(3, 1),
            "expected 'this', a relation name or '(', found the end",
        ),
        (
            "namespace: t\n  owner<>\n",
            place(2, 9),
            "expected a constraint, found '>'",
        ),
    ] {
        let error = only(text);
        assert_eq!(error.place, at, "{text:?}");
        assert_eq!(error.kind.to_string(), message, "{text:?}");
    }
}

#[test]
fn a_rewrite_can_start_on_the_next_line_and_wrap() {
    let one_line = one("namespace: t\n  a<@identity>\n  r<@identity>: this | a ! a\n");
    let cases = [
        "namespace: t\n  a<@identity>\n  r<@identity>:\n    this | a ! a\n",
        "namespace: t\n  a<@identity>\n  r<@identity>\n    : this\n    | a\n    ! a\n",
        "namespace: t a<@identity> r<@identity>: this | a ! a",
        "namespace:t\r\n\ta<@identity>\r\n\tr<\r\n@identity\r\n>:this|a!a",
    ];
    for text in cases {
        assert_eq!(
            sorted(&one(text).relations),
            sorted(&one_line.relations),
            "{text:?}"
        );
    }
}

/// `drive`'s `file` and `folder`, as the namespace fixtures declare them.
const DRIVE: &str = "theory: drive

// Files sit in folders, and a file's viewers include its folder's.
namespace: file
  owner<@identity>
  parent<@identity, folder, folder#owner>
  editor<..group#member, @identity>: this | owner
  viewer<..group#member, @identity>: (this | editor | (parent, viewer)) ! banned
  auditor<@identity>: this & viewer
  banned<@identity>

namespace: folder
  owner<@identity>
  parent<folder>
  viewer<..group#member, @identity>: (this | (parent, viewer)) ! banned
  banned<..group#member, @identity>
";

const FIXTURES: [&str; 3] = [
    include_str!("../../sungma/tests/fixtures/namespaces/group.theory"),
    include_str!("../../sungma/tests/fixtures/namespaces/folder.theory"),
    include_str!("../../sungma/tests/fixtures/namespaces/file.theory"),
];

#[test]
fn comments_change_nothing() {
    let plain = parsed(DRIVE);
    let commented = DRIVE
        .replace(
            "\nnamespace: folder",
            "// folders\nnamespace: folder // and more\n",
        )
        .replace(
            "  banned<@identity>\n",
            "  banned<@identity> // é \u{202e} //\n",
        )
        .replace(
            "editor<..group#member, @identity>: this | owner",
            "editor<@identity, // who\n    ..group#member>: this // first\n    | owner",
        );
    // At the end of a `theory:` line.
    let commented = commented.replacen("theory: drive\n", "theory: drive // the drive\n", 1);
    assert!(commented.starts_with("theory: drive // the drive\n"));
    let commented = parsed(&format!("// top\n{commented}// last"));
    assert!(same(&plain, &commented));
}

#[test]
fn what_isnt_in_the_grammar_is_refused() {
    let cases = [
        ("namespace: t\n  r: \"this\"", 2, 6, "'\"'"),
        ("namespace: t\n  r: 'this'", 2, 6, "'''"),
        ("namespace: t\n  r: 1", 2, 6, "'1'"),
        ("namespace: t\n  r<@identity>: this # comment", 2, 22, "'#'"),
        ("namespace: t\n  r<@identity>: this / comment", 2, 22, "'/'"),
        ("namespace: t\n  r<@identity> / comment", 2, 16, "'/'"),
        ("namespace: t\n  r: é", 2, 6, "'é'"),
        ("namespace: t\n  r: \0", 2, 6, "'\\u{0000}'"),
        ("namespace: t\n  r = this", 2, 3, "'r'"),
        ("namespace: t\n  r: a\\u0041", 2, 7, "'\\'"),
        ("# t\nnamespace: t", 1, 1, "'#'"),
        ("namespace: t\n  # r<@identity>", 2, 3, "'#'"),
        (
            "namespace: t\n  r<@identity>: this\u{feff}",
            2,
            21,
            "'\\u{feff}'",
        ),
    ];
    for (text, line, column, found) in cases {
        let error = only(text);
        assert_eq!(error.place, place(line, column), "{text:?}: {error}");
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
    assert_eq!(
        error.to_string(),
        "a document doesn't start with a byte-order mark"
    );
}

#[test]
fn a_rewrite_left_open_doesnt_swallow_what_comes_next() {
    for text in [
        "namespace: a\n  r<@identity>: this |\nnamespace: b\n  r<@identity>",
        "namespace: a\n  r<@identity>: this |\n  b<@identity>\n  c: b",
        "namespace: a\n  r<@identity>: this !\n  s<@identity>: this",
        "namespace: a\n  r<@identity>: this |\ntheory: t\nnamespace: b\n",
        "namespace: x\n  b<@identity>\n  r: (b\n  a<@identity>\n  c: a\n",
        "namespace: x\n  b<@identity>\n  r: (b,\n  a<@identity>\n  c: a\n",
    ] {
        let error = only(text);
        assert_eq!(error.relation.as_deref(), Some("r"), "{text:?}");
    }
}

#[test]
fn a_missing_name_doesnt_take_what_comes_next() {
    for (text, found) in [
        ("theory:\nnamespace: x\n  a<@identity>\n", "'namespace'"),
        ("namespace:\n  owner<@identity>\n", "'owner'"),
        ("namespace:\n  owner: a\n  a<@identity>\n", "'owner'"),
        ("namespace:\nnamespace: x\n", "'namespace'"),
    ] {
        let error = only(text);
        assert!(
            error.kind.to_string().ends_with(&format!("found {found}")),
            "{text:?}: {error}"
        );
    }
    let errors = refused("namespace: !\nnamespace: !\n");
    assert!(
        errors
            .iter()
            .all(|error| !matches!(error.kind, ErrorKind::DuplicateNamespace(_))),
        "{errors:?}"
    );
}

#[test]
fn one_bad_namespace_refuses_the_document() {
    let text = format!("{DRIVE}\nnamespace: other\n  r: undeclared\n");
    let error = only(&text);
    assert_eq!(error.namespace.as_deref(), Some("drive.other"));
    assert_eq!(error.relation.as_deref(), Some("r"));
    assert_eq!(error.place, place(19, 3));
    assert!(
        matches!(
            error.kind,
            ErrorKind::Namespace(NamespaceError::DanglingReference { .. })
        ),
        "{error}"
    );
}

#[test]
fn relations_belong_to_a_namespace() {
    let error = only("theory: t\n  r<@identity>\nnamespace: n\n");
    assert_eq!(error.namespace, None);
    assert_eq!(error.place, place(2, 3));
    assert_eq!(
        error.kind.to_string(),
        "expected 'namespace:', 'theory:' or the end, found 'r'"
    );
}

#[test]
fn a_namespace_declared_twice_is_refused() {
    for text in [
        "namespace: a\nnamespace: b\nnamespace: a\n",
        "theory: t\nnamespace: a\nnamespace: a\n",
        "namespace: t.a.b\ntheory: t\nnamespace: a.b\n",
        "namespace: t.a\ntheory: t\nnamespace: a\n",
        "theory: t\nnamespace: a\ntheory: u\nnamespace: b\ntheory: t\nnamespace: a\n",
    ] {
        let errors = refused(text);
        let error = errors.last().expect("an error");
        assert_eq!(errors.len(), 1, "{text:?}: {errors:?}");
        assert!(
            matches!(error.kind, ErrorKind::DuplicateNamespace(_)),
            "{text:?}: {error}"
        );
    }
}

#[test]
fn namespace_checks_apply_to_each_namespace() {
    let errors =
        refused("namespace: a\n  r: s\n  s: r\nnamespace: b\n  r<@identity>\n  r<@identity>\n");
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert_eq!(errors[0].namespace.as_deref(), Some("a"));
    assert!(matches!(
        errors[0].kind,
        ErrorKind::Namespace(NamespaceError::RewriteCycle { .. })
    ));
    assert_eq!(errors[1].namespace.as_deref(), Some("b"));
    assert_eq!(errors[1].place, place(6, 3));
    assert!(matches!(
        errors[1].kind,
        ErrorKind::Namespace(NamespaceError::DuplicateRelation(_))
    ));
}

#[test]
fn a_reference_between_namespaces_is_checked_against_the_document() {
    // Either order, by constraint or by a fact-to-subjectset.
    for text in [
        "namespace: a\n  r<b#m>\nnamespace: b\n  m<@identity>\n",
        "namespace: b\n  m<@identity>\nnamespace: a\n  r<b#m>\n",
        "namespace: a\n  p<b>\n  v: (p, m)\nnamespace: b\n  m<@identity>\n",
        "theory: t\nnamespace: a\n  r<b#m>\nnamespace: b\n  m<@identity>\n",
    ] {
        parsed(text);
    }
    let error = only("namespace: a\n  r<b#x>\nnamespace: b\n  m<@identity>\n");
    assert_eq!(error.namespace.as_deref(), Some("a"));
    assert_eq!(error.place, place(2, 3));
    assert_eq!(
        error.kind.to_string(),
        "relation 'r' has constraint 'b#x', whose namespace doesn't declare its relation"
    );
    let error =
        only("namespace: b\n  m<@identity>\nnamespace: a\n  p<b, @identity>\n  v: (p, x)\n");
    assert_eq!(error.relation.as_deref(), Some("v"));
    assert_eq!(
        error.kind.to_string(),
        "relation 'v' follows 'x', which namespace 'b' doesn't declare"
    );
    // A namespace outside the document is left to the namespace write.
    parsed("namespace: a\n  r<elsewhere#x>\n  p<elsewhere>\n  v: (p, x)\n");
}

// The theory name.

#[test]
fn the_theory_prefixes_every_name_in_headers_and_constraints() {
    let document = one(
        "theory: t\nnamespace: a\n  r<@identity, b, b#s, t.c, t.c#s, u.d, u.d#s>: this | s\n  s<@identity>\n",
    );
    assert_eq!(document.namespace.as_str(), "t.a");
    assert_eq!(
        constraints_of(&document, "r"),
        [
            "@identity",
            "t.b",
            "t.b#s",
            "t.t.c",
            "t.t.c#s",
            "t.u.d",
            "t.u.d#s"
        ]
    );
    assert_eq!(rewrite_of(&document, "r"), union(vec![This, name("s")]));
    let documents = parsed("theory: t\nnamespace: c.d\nnamespace: e\nnamespace: t.f\n");
    assert_eq!(names(&documents), ["t.c.d", "t.e", "t.t.f"]);
}

#[test]
fn a_document_of_plain_namespaces_and_theories_resolves_each_name() {
    let text = "namespace: a
  r<@identity>
namespace: b
  r<@identity>

theory: c

namespace: d
  r<@identity>

theory: x

namespace: y.z
  r<@identity>
";
    let documents = parsed(text);
    assert_eq!(names(&documents), ["a", "b", "c.d", "x.y.z"]);
    // Printed in any order, they print as written.
    let mut shuffled = parsed(text);
    shuffled.reverse();
    shuffled.swap(2, 3);
    assert_eq!(names(&shuffled), ["x.y.z", "c.d", "a", "b"]);
    assert_eq!(print(&shuffled), Ok(text.to_owned()));
}

#[test]
fn dots_before_a_constraint_name_it_outside_the_theory() {
    let document = one(
        "theory: drive\nnamespace: folder\n  r<file, ..file, ..io.file#owner, ..drive.x#owner, file#owner, drive.file>\n",
    );
    assert_eq!(document.namespace.as_str(), "drive.folder");
    assert_eq!(
        constraints_of(&document, "r"),
        [
            "drive.drive.file",
            "drive.file",
            "drive.file#owner",
            "drive.x#owner",
            "file",
            "io.file#owner",
        ]
    );
    // Outside any theory, `..` changes nothing.
    let document = one("namespace: a\n  r<..file, ..io.file#owner>\n");
    assert_eq!(constraints_of(&document, "r"), ["file", "io.file#owner"]);
    assert_eq!(
        tdl::constraint("..io.file#owner"),
        tdl::constraint("io.file#owner")
    );
    // `..` reads only at the start of a constraint, never in a header or
    // inside a name.
    for (text, quoted) in [
        ("theory: t\nnamespace: a\n  r<a..b>\n", "a..b"),
        ("theory: t\nnamespace: a\n  r<...b>\n", "...b"),
        ("theory: t\nnamespace: a\n  r<..b.>\n", "..b."),
        ("theory: t\nnamespace: a\n  r<..>\n", ".."),
        ("theory: t\nnamespace: a\n  r<..@identity>\n", "..@identity"),
        ("theory: t\nnamespace: a\n  r<b#..c>\n", "b#..c"),
        ("theory: t\nnamespace: a\n  r<.. b>\n", ".."),
    ] {
        let error = only(text);
        assert_eq!(error.place, place(3, 5), "{text:?}: {error}");
        assert!(error.to_string().contains(quoted), "{text:?}: {error}");
    }
    for text in ["namespace: ..a\n", "theory: t\nnamespace: ..a\n"] {
        let error = only(text);
        assert!(
            matches!(error.kind, ErrorKind::Name(NameError::Invalid(_))),
            "{text:?}: {error}"
        );
        assert_eq!(error.place, place(text.lines().count(), 12), "{text:?}");
    }
    assert!(matches!(
        only("theory: ..t\nnamespace: a\n").kind,
        ErrorKind::DottedTheory(_)
    ));
}

#[test]
fn a_rewrite_ends_at_the_next_theory_line() {
    let documents = parsed(
        "namespace: a\n  r<@identity>: this |\n    s\n  s<@identity>\ntheory: t\nnamespace: b\n  s<@identity>: this\ntheory: u\nnamespace: c\n",
    );
    assert_eq!(names(&documents), ["a", "t.b", "u.c"]);
    assert_eq!(rewrite_of(&documents[0], "r"), union(vec![This, name("s")]));
    assert_eq!(rewrite_of(&documents[1], "s"), This);
    // A rewrite left open stops at `theory:`, which reads on as a header.
    let error = only("namespace: a\n  r<@identity>: this |\ntheory: t\nnamespace: b\n");
    assert_eq!(error.place, place(3, 1));
    assert!(error.to_string().contains("found the end"), "{error}");
}

#[test]
fn each_theory_line_holds_until_the_next() {
    let documents = parsed(
        "namespace: a\n  r<b>\nnamespace: x.b\ntheory: t\nnamespace: a\n  r<b>\ntheory: u\nnamespace: a\n  r<b>\nnamespace: b\n",
    );
    assert_eq!(names(&documents), ["a", "x.b", "t.a", "u.a", "u.b"]);
    let constraints: Vec<_> = documents
        .iter()
        .filter(|document| document.namespace.as_str().ends_with('a'))
        .map(|document| constraints_of(document, "r"))
        .collect();
    assert_eq!(constraints, [["b"], ["t.b"], ["u.b"]]);
}

#[test]
fn a_theory_name_is_one_name() {
    let error = only("theory: a.b\nnamespace: c\nnamespace: d\n");
    assert_eq!(error.place, place(1, 9));
    assert_eq!(
        error.kind.to_string(),
        "theory 'a.b' isn't one name: a theory name has no dot"
    );
    for (text, at) in [
        ("theory: 1a\nnamespace: c\n", place(1, 9)),
        ("theory: a-b\nnamespace: c\n  r<d>\n", place(1, 9)),
        ("theory: a b\nnamespace: c\n", place(1, 11)),
    ] {
        let error = only(text);
        assert_eq!(error.place, at, "{text:?}: {error}");
    }
    // Under a theory whose name failed, a namespace name is still checked
    // as written.
    let error = refused("theory: a..b\nnamespace: 1c\n");
    assert_eq!(error.len(), 2, "{error:?}");
    assert_eq!(error[1].place, place(2, 12));
}

#[test]
fn a_theory_declares_at_least_one_namespace() {
    for (text, line) in [
        ("theory: t\n", 1),
        ("theory: t\n// nothing\ntheory: u\nnamespace: a\n", 1),
        ("namespace: a\ntheory: t\n", 2),
    ] {
        let error = only(text);
        assert_eq!(error.place, place(line, 1), "{text:?}");
        assert_eq!(
            error.kind.to_string(),
            "theory 't' declares no namespace",
            "{text:?}"
        );
    }
}

#[test]
fn identity_is_a_sigil_so_it_names_namespaces() {
    let documents = parsed(
        "namespace: identity
  owner<@identity, identity, identity#owner>
theory: identity
namespace: identity
",
    );
    assert_eq!(names(&documents), ["identity", "identity.identity"]);
    assert_eq!(
        constraints_of(&documents[0], "owner"),
        ["@identity", "identity", "identity#owner"]
    );
    for (text, message) in [
        (
            "namespace: t
  r<@identity#member>
",
            "'@identity#member' is not a valid name",
        ),
        (
            "namespace: t
  r<@other>
",
            "'@other' is not a valid name",
        ),
        (
            "namespace: t
  r<@ identity>
",
            "'@' is not a valid name",
        ),
    ] {
        let error = only(text);
        assert_eq!(error.place, place(2, 5), "{text:?}");
        assert!(error.to_string().ends_with(message), "{text:?}: {error}");
    }
}

// Names.

#[test]
fn names_follow_the_grammar() {
    for text in [
        "namespace: .a",
        "namespace: a.",
        "namespace: a..b",
        "namespace: a.1b",
        "namespace: 1a",
        "namespace: my-ns",
        "namespace: a\"b",
        "namespace: a#b",
    ] {
        let error = refused(text).remove(0);
        assert_eq!(error.place, place(1, 12), "{text:?}: {error}");
        assert!(
            matches!(error.kind, ErrorKind::Name(NameError::Invalid(_))),
            "{text:?}: {error}"
        );
    }
    for constraint in [
        ".a", "a.", "a..b", "a.1b", "a#", "#a", "a#b#c", "a#1", "a\u{7}",
    ] {
        let text = format!("namespace: t\n  r<{constraint}>\n");
        let error = only(&text);
        assert_eq!(error.place, place(2, 5), "{text:?}");
        assert!(
            matches!(error.kind, ErrorKind::Name(NameError::Invalid(_))),
            "{text:?}: {error}"
        );
    }
    // A name written with spaces is two names.
    let error = only("namespace: a .b");
    assert_eq!(error.place, place(1, 14));
    let error = only("namespace: t\n  r<a #b>\n");
    assert_eq!(error.kind.to_string(), "expected ',' or '>', found '#'");
}

#[test]
fn checked_names_refuse_a_colon() {
    for name in ["a:b", ":a", "a:"] {
        assert!(name.parse::<NamespaceName>().is_err(), "{name:?}");
        assert!(name.parse::<RelationName>().is_err(), "{name:?}");
        assert!(tdl::constraint(name).is_err(), "{name:?}");
        assert!(tdl::constraint(&format!("t#{name}")).is_err(), "{name:?}");
    }
    for text in [
        "namespace: a:b\n",
        "theory: t\nnamespace: a:b\n",
        "theory: a:b\nnamespace: c\n",
        "namespace: t\n  r:s<@identity>\n",
        "namespace: t\n  r<a:b>\n",
        "namespace: t\n  r<t#a:b>\n",
    ] {
        refused(text);
    }
}

#[test]
fn a_malformed_relation_name_is_refused_whole() {
    for (written, tail) in [
        ("my-rel", "rel"),
        ("owner#x", "x"),
        ("a.b", "b"),
        ("x@y", "y"),
    ] {
        // The name's tail declared again would be a duplicate, were it
        // declared from the malformed name.
        for text in [
            format!("namespace: t\n  {written}<@identity>\n"),
            format!("namespace: t\n  {written}<@identity>\n  {tail}<@identity>\n"),
            format!("namespace: t\n  v<@identity>: this\n  {written}: v\n  {tail}: v\n"),
        ] {
            let error = only(&text);
            assert_eq!(
                error.place,
                place(2 + text.matches("v<").count(), 3),
                "{text:?}"
            );
            assert_eq!(
                error.kind.to_string(),
                format!("'{written}' is not a valid name"),
                "{text:?}"
            );
            assert_eq!(error.relation.as_deref(), Some(written), "{text:?}");
        }
    }
}

#[test]
fn names_are_case_sensitive() {
    let document = one("namespace: t\n  a<@identity>\n  A<@identity>\n  r: a | A\n");
    assert_eq!(document.relations.relations().count(), 3);
    assert_eq!(names(&parsed("namespace: T\nnamespace: t\n")), ["T", "t"]);
}

#[test]
fn keywords_arent_relation_names_but_name_namespaces() {
    for (text, name) in [
        ("namespace: t\n  theory<@identity>\n", "theory"),
        ("namespace: t\n  a<@identity>\n  Theory: a\n", "Theory"),
        (
            "namespace: t\n  a<@identity>\n  Namespace: a\n",
            "Namespace",
        ),
        ("namespace: t\n  THEORY<@identity>\n", "THEORY"),
        ("namespace: t\n  This<@identity>\n", "This"),
        ("namespace: t\n  this<@identity>\n", "this"),
        ("namespace: t\n  a<@identity>\n  r: a | theory\n", "theory"),
        ("namespace: t\n  r<t#namespace>\n", "namespace"),
    ] {
        let error = only(text);
        assert!(
            matches!(error.kind, ErrorKind::Name(NameError::Reserved(_))),
            "{text}: {error}"
        );
        assert!(error.to_string().contains(name), "{error}");
    }
    let documents = parsed(
        "theory: sungma\nnamespace: namespace\n  parent<theory, namespace#parent>\nnamespace: theory\n",
    );
    assert_eq!(names(&documents), ["sungma.namespace", "sungma.theory"]);
    assert_eq!(
        constraints_of(&documents[0], "parent"),
        ["sungma.namespace#parent", "sungma.theory"]
    );
    let documents = parsed("theory: namespace\nnamespace: theory\n");
    assert_eq!(names(&documents), ["namespace.theory"]);
}

#[test]
fn the_parser_reads_sungma_s_own_namespaces() {
    let documents = parsed("namespace: sungma.audit\ntheory: sungma\nnamespace: root\n");
    assert_eq!(names(&documents), ["sungma.audit", "sungma.root"]);
}

// Checks.

#[test]
fn each_check_refuses_what_it_should() {
    for (text, check) in [
        (
            "namespace: t\n  r<@identity>\n  r<@identity>\n",
            "is declared more than once",
        ),
        ("namespace: t\n  r: x\n", "references 'x'"),
        ("namespace: t\n  r: (x, y)\n", "references 'x'"),
        (
            "namespace: t\n  r: this\n",
            "reads 'this' and declares no constraints",
        ),
        (
            "namespace: t\n  a<@identity>\n  r<@identity>: a\n",
            "doesn't read 'this'",
        ),
        (
            "namespace: t\n  r<t#x>\n",
            "whose namespace doesn't declare its relation",
        ),
        (
            "namespace: t\n  p<t>\n  r: (p, x)\n",
            "follows 'x', which namespace 't'",
        ),
        (
            "namespace: t\n  a: b\n  b: a\n",
            "rewrite cycle: a -> b -> a",
        ),
        (
            "namespace: t\n  r<@identity>: this ! r\n",
            "rewrite cycle: r -> r",
        ),
        (
            "namespace: t\n  r<@identity, @identity>\n",
            "declares constraint '@identity' more than once",
        ),
    ] {
        let error = only(text);
        assert!(
            matches!(error.kind, ErrorKind::Namespace(_)),
            "{text:?}: {error}"
        );
        assert!(error.to_string().contains(check), "{text:?}: {error}");
    }
    // Cycles through a fact-to-subjectset, and relations sharing a target,
    // are allowed.
    parsed("namespace: folder\n  parent<folder>\n  viewer<@identity>: this | (parent, viewer)\n");
    parsed("namespace: t\n  a<@identity>\n  b: a\n  c: a | b\n");
}

#[test]
fn a_dangling_target_is_reported_once_per_relation_that_names_it() {
    let errors = refused("namespace: t\n  a: x | x\n  b<@identity>: this | x & (x, y)\n");
    let relations: Vec<_> = errors.iter().map(|e| e.relation.as_deref()).collect();
    assert_eq!(relations, [Some("a"), Some("b")], "{errors:?}");
    let error = &errors[1];
    assert_eq!(error.place, place(3, 3));
}

#[test]
fn relations_that_reach_each_other_are_one_cycle() {
    let error = only("namespace: t\n  a: b\n  b: c | a\n  c: a\n");
    assert_eq!(error.relation.as_deref(), Some("a"));
    let ErrorKind::Namespace(NamespaceError::RewriteCycle { path, .. }) = error.kind else {
        panic!("expected a cycle, got {error}");
    };
    assert_eq!(path, ["a", "b", "a"]);
}

#[test]
fn a_long_cycle_names_every_relation_on_its_path() {
    let count = 20;
    let ring: String = (0..count)
        .map(|i| format!("  r{i}: r{}\n", (i + 1) % count))
        .collect();
    let error = only(&format!("namespace: t\n{ring}"));
    let ErrorKind::Namespace(NamespaceError::RewriteCycle { path }) = error.kind else {
        panic!("expected a cycle, got {error}");
    };
    let every: Vec<_> = (0..count).chain([0]).map(|i| format!("r{i}")).collect();
    assert_eq!(path, every);
}

#[test]
fn a_constraint_list_can_wrap_and_holds_any_number_of_constraints() {
    let wrapped = one("namespace: t\n  owner<@identity,\n    group#member,\n    folder>\n");
    assert_eq!(
        constraints_of(&wrapped, "owner"),
        ["@identity", "folder", "group#member"]
    );
    // There's no cap on constraints.
    let many: Vec<_> = (0..100).map(|i| format!("n{i}")).collect();
    let document = one(&format!("namespace: t\n  owner<{}>\n", many.join(", ")));
    assert_eq!(constraints_of(&document, "owner").len(), 100);
}

#[test]
fn a_malformed_constraint_list_is_one_error_where_it_goes_wrong() {
    for (text, line, column, message) in [
        (
            "namespace: t\n  owner<@identity,\n",
            3,
            1,
            "expected a constraint, found the end",
        ),
        (
            "namespace: t\n  owner<, @identity>\n",
            2,
            9,
            "expected a constraint, found ','",
        ),
        (
            "namespace: t\n  owner<@identity, >\n",
            2,
            20,
            "expected a constraint, found '>'",
        ),
        (
            "namespace: t\n  owner<(a)>\n",
            2,
            9,
            "expected a constraint, found '('",
        ),
        (
            "namespace: t\n  owner<a b>\n",
            2,
            11,
            "expected ',' or '>', found 'b'",
        ),
        (
            "namespace: t\n  owner<a\n  viewer<@identity>\n",
            3,
            3,
            "expected ',' or '>', found 'viewer'",
        ),
        (
            "namespace: t\n  owner<a>>\n",
            2,
            11,
            "expected ':', a relation, 'namespace:', 'theory:' or the end, found '>'",
        ),
        (
            "namespace: t\n  owner<a#>\n",
            2,
            9,
            "'a#' is not a valid name",
        ),
        (
            "namespace: t\n  owner<a#this>: this\n",
            2,
            9,
            "'this' is reserved",
        ),
    ] {
        let error = only(text);
        assert_eq!(error.relation.as_deref(), Some("owner"), "{text:?}");
        assert_eq!(error.place, place(line, column), "{text:?}: {error}");
        assert!(
            error.kind.to_string().ends_with(message),
            "{text:?}: {error}"
        );
    }
}

#[test]
fn a_bad_constraint_doesnt_read_a_later_one_as_a_relation() {
    for text in [
        "namespace: group\n  member<@identity>\n  editor<!, group#member>: this\n",
        "namespace: t\n  editor<!>: a\n  a<@identity>\n",
        "namespace: t\n  editor<a b c d>: this\n",
    ] {
        let errors = refused(text);
        assert_eq!(errors.len(), 1, "{text:?}: {errors:?}");
        assert_eq!(errors[0].relation.as_deref(), Some("editor"), "{text:?}");
    }
    let errors = refused("namespace: t\n  editor<!\n  viewer<!>\n");
    let relations: Vec<_> = errors.iter().map(|e| e.relation.as_deref()).collect();
    assert_eq!(relations, [Some("editor"), Some("viewer")]);
}

#[test]
fn a_relation_whose_rewrite_fails_is_still_declared() {
    let error = only("namespace: t\n  a<@identity>: this (\n  b: a\n");
    assert_eq!(error.relation.as_deref(), Some("a"));
    let error = only("namespace: t\n  a: (\n  b: a\n");
    assert_eq!(error.relation.as_deref(), Some("a"));
}

// Errors.

#[test]
fn every_problem_is_reported_in_document_order_once() {
    let text = "namespace: t
  a<@identity>: this |
  b<@identity>
  c<@identity>: this )
  d: undeclared
  e<@identity>: (this
  f<@identity>
";
    let errors = refused(text);
    let places: Vec<_> = errors
        .iter()
        .map(|error| {
            (
                error.relation.as_deref(),
                error.place.map(|place| place.line),
            )
        })
        .collect();
    assert_eq!(
        places,
        [
            (Some("a"), Some(3)),
            (Some("c"), Some(4)),
            (Some("d"), Some(5)),
            (Some("e"), Some(7)),
        ],
        "{errors:?}"
    );
    let messages = || -> Vec<String> { refused(text).iter().map(ToString::to_string).collect() };
    let first = messages();
    for _ in 0..10 {
        assert_eq!(messages(), first);
    }
}

/// `text`'s errors, with the error cap set to `cap`.
fn capped(text: &str, cap: usize) -> Vec<TdlError> {
    let settings = Settings {
        errors: cap,
        ..Settings::default()
    };
    tdl::parse_with(text, settings).expect_err("a refused document")
}

/// `count` relations naming an undeclared one, an error each.
fn undeclared(count: usize) -> String {
    (0..count)
        .map(|i| format!("  r{i}: undeclared\n"))
        .collect()
}

fn too_many(error: &TdlError, cap: usize) -> bool {
    matches!(error.kind, ErrorKind::TooManyErrors(at) if at == cap) && error.place.is_none()
}

#[test]
fn errors_stop_at_the_cap() {
    assert_eq!(Settings::default().errors, DEFAULT_MAX_ERRORS);
    assert_eq!(DEFAULT_MAX_ERRORS, 50);
    let errors = refused(&format!("namespace: t\n{}", undeclared(55)));
    assert_eq!(errors.len(), DEFAULT_MAX_ERRORS + 1);
    assert!(too_many(&errors[DEFAULT_MAX_ERRORS], DEFAULT_MAX_ERRORS));
    assert_eq!(
        errors[DEFAULT_MAX_ERRORS - 1].relation.as_deref(),
        Some("r49")
    );
    assert_eq!(
        errors[DEFAULT_MAX_ERRORS].to_string(),
        "too many errors; the first 50 are reported"
    );
    // With the cap at 1, two errors report the first and that there were
    // more.
    let errors = capped("namespace: t\n  a: x\n  b: y\n", 1);
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert_eq!(errors[0].relation.as_deref(), Some("a"));
    assert!(too_many(&errors[1], 1), "{errors:?}");
    // As many errors as the cap are all reported, with nothing more.
    for cap in [1, 3, 7] {
        let errors = capped(&format!("namespace: t\n{}", undeclared(cap)), cap);
        assert_eq!(errors.len(), cap);
        assert!(
            errors
                .iter()
                .all(|error| !matches!(error.kind, ErrorKind::TooManyErrors(_)))
        );
    }
    // Errors from a later namespace count toward the cap.
    let text = format!(
        "namespace: a\n{}namespace: b\n  r: undeclared\n",
        undeclared(3)
    );
    let errors = capped(&text, 3);
    assert_eq!(errors.len(), 4);
    assert!(too_many(&errors[3], 3));
    // Reading stops once past the cap.
    let many: String = (0..10 * DEFAULT_MAX_ERRORS)
        .map(|i| format!("namespace: n{i}\n  r: undeclared\n"))
        .collect();
    assert_eq!(refused(&many).len(), DEFAULT_MAX_ERRORS + 1);
}

#[test]
fn errors_past_the_cap_keep_the_earliest_in_document_order() {
    // Each line is a relation whose constraint list fails, and declares the
    // relation again: two errors a line, the syntax error found first and
    // the duplicate found later by the namespace's checks, so errors arrive
    // out of document order.
    let text = format!("namespace: t\n{}", "  a<\n".repeat(1000));
    for cap in [1, 2, 5, 50] {
        let errors = capped(&text, cap);
        assert_eq!(errors.len(), cap + 1, "{cap}");
        assert!(too_many(&errors[cap], cap));
        for (at, error) in errors[..cap].iter().enumerate() {
            assert_eq!(error.place, place(3 + at / 2, 3), "{cap}: {error}");
            if at % 2 == 0 {
                assert!(matches!(error.kind, ErrorKind::Syntax { .. }), "{error}");
            } else {
                assert!(matches!(error.kind, ErrorKind::Namespace(_)), "{error}");
            }
        }
    }
}

#[test]
fn errors_name_their_place_and_namespace() {
    let error = only("namespace: t\n  // é\n  r: é");
    assert_eq!(
        error.to_string(),
        "line 3, column 6: namespace 't', relation 'r': \
         expected 'this', a relation name or '(', found 'é'"
    );
    let error = only("theory: io\nnamespace: t\n  r<@identity>\n  r<@identity>");
    assert_eq!(
        error.to_string(),
        "line 4, column 3: namespace 'io.t', relation 'r': relation 'r' is declared more than once"
    );
    let error = only("theory: t\nnamespace: a\nnamespace: a");
    assert_eq!(
        error.to_string(),
        "line 3, column 1: namespace 't.a': namespace 't.a' is declared twice"
    );
    let error = only("theory: t\nnamespace: 1a");
    assert_eq!(
        error.to_string(),
        "line 2, column 12: namespace 't.1a': 't.1a' is not a valid name"
    );
}

#[test]
fn columns_count_characters_on_their_line() {
    let error = only("namespace: t\n  a<@identity> // ü ü\n  r<@identity>: this | ü");
    assert_eq!(error.place, place(3, 24));
    let error = only("namespace: t\n  é<@identity>");
    assert_eq!(error.place, place(2, 3));
    let error = only("namespace: t\n  r<é, ü>");
    assert_eq!(error.place, place(2, 5));
    let error = only("namespace: t\n  r<@identity>: this\n    | this\n    ! ü");
    assert_eq!(error.place, place(4, 7));
    let error = only("namespace: t\n  r: 😀 | a");
    assert_eq!(error.place, place(2, 6));
    let error = only("namespace: t\n  a<@identity>\n  r: ((a) | a");
    assert_eq!(error.place, place(3, 14));
    let error = only("namespace: t\n  r: // note");
    assert_eq!(error.place, place(2, 13));
}

#[test]
fn an_error_quotes_a_bounded_and_escaped_cut() {
    let long = "n".repeat(1000);
    let error = only(&format!("namespace: t\n  {long}<@identity>"));
    let quoted = format!("{}...", "n".repeat(MAX_QUOTE_BYTES - 3));
    assert_eq!(error.relation.as_deref(), Some(quoted.as_str()));
    assert!(matches!(&error.kind, ErrorKind::Name(NameError::TooLong(name, _)) if *name == quoted));
    // A cut never splits a character: 20 three-byte characters fill 60
    // bytes, and a 21st leaves no room for the cut mark.
    let wide = "€".repeat(30);
    let error = only(&format!("namespace: {wide}"));
    let ErrorKind::Name(NameError::Invalid(name)) = &error.kind else {
        panic!("expected an invalid name, got {error}");
    };
    assert_eq!(*name, format!("{}...", "€".repeat(20)));
    let error = only(&format!("namespace: {long}"));
    assert!(error.to_string().len() < 300, "{error}");
    let error = only(&format!("namespace: t\n  r<{long}#a>"));
    assert!(error.to_string().len() < 300, "{error}");
}

#[test]
fn an_error_escapes_hidden_characters() {
    for code in [0x07, 0x1b, 0x7f, 0x9b, 0x200b, 0x202e, 0xfeff, 0x2066] {
        let c = char::from_u32(code).expect("a character");
        for text in [
            format!("namespace: t\n  r<@identity>: a{c}b"),
            format!("namespace: t\n  a: a | {c}"),
            format!("namespace: t{c}"),
            format!("namespace: t\n  a<{c}>"),
        ] {
            let error = refused(&text).remove(0);
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
fn a_refusal_joins_its_errors() {
    let refusal = Refusal(refused("namespace: t\n  a: x\n  b: y\n"));
    assert_eq!(
        refusal.to_string(),
        "line 2, column 3: namespace 't', relation 'a': relation 'a' references 'x', which the namespace doesn't declare; \
         line 3, column 3: namespace 't', relation 'b': relation 'b' references 'y', which the namespace doesn't declare"
    );
}

// One namespace.

#[test]
fn parse_one_takes_a_document_of_one_namespace() {
    let document = tdl::parse_one("theory: t\nnamespace: a\n").expect("one namespace");
    assert_eq!(document.namespace.as_str(), "t.a");
    let errors = tdl::parse_one("namespace: a\nnamespace: b\n").expect_err("two namespaces");
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].to_string(),
        "a document here declares one namespace, not 2"
    );
    let errors = tdl::parse_one("namespace: a\n  r: x\n").expect_err("a refused namespace");
    assert!(matches!(errors[0].kind, ErrorKind::Namespace(_)));
}

#[test]
fn a_constraint_reads_as_written_outside_a_theory() {
    assert_eq!(tdl::constraint("@identity"), Ok(Constraint::Identity));
    assert_eq!(
        tdl::constraint("drive.folder"),
        Ok(Constraint::Resource(namespace_name("drive.folder")))
    );
    assert_eq!(
        tdl::constraint("group#member"),
        Ok(Constraint::Subjectset(
            namespace_name("group"),
            relation("member")
        ))
    );
    assert_eq!(
        tdl::constraint("identity"),
        Ok(Constraint::Resource(namespace_name("identity")))
    );
    for text in ["a b", "a#this", "@identity#a", "@other", ""] {
        assert!(tdl::constraint(text).is_err(), "{text:?}");
    }
}

// Limits.

#[test]
fn a_document_has_no_size_limit() {
    // Request size is the edge's to limit; a stored version is read back
    // whatever its size.
    let text = format!("{}namespace: t", " ".repeat(2 * LARGE));
    assert_eq!(
        tdl::parse_one(&text)
            .expect("no size limit")
            .namespace
            .as_str(),
        "t"
    );
}

#[test]
fn names_hold_at_their_limits() {
    let name = "n".repeat(MAX_NAMESPACE_BYTES - 2);
    one(&format!("theory: t\nnamespace: {name}"));
    let error = only(&format!("theory: tt\nnamespace: {name}"));
    assert!(
        matches!(error.kind, ErrorKind::Name(NameError::TooLong(..))),
        "{error}"
    );
    let dotted = format!("a.{}", "n".repeat(MAX_NAMESPACE_BYTES - 2));
    assert_eq!(
        one(&format!("namespace: {dotted}")).namespace.as_str(),
        dotted
    );
    let error = only(&format!("namespace: {dotted}n"));
    assert!(
        matches!(error.kind, ErrorKind::Name(NameError::TooLong(..))),
        "{error}"
    );
    // Under a theory, the limit is on the full name, prefix and all.
    let rest = format!("b.{}", "n".repeat(MAX_NAMESPACE_BYTES - 4));
    assert_eq!(
        one(&format!("theory: a\nnamespace: {rest}"))
            .namespace
            .as_str()
            .len(),
        MAX_NAMESPACE_BYTES
    );
    let error = only(&format!("theory: a\nnamespace: {rest}n"));
    assert!(
        matches!(error.kind, ErrorKind::Name(NameError::TooLong(..))),
        "{error}"
    );
    let error = only(&format!("theory: a\nnamespace: b\n  r<{rest}n>\n"));
    assert!(
        matches!(error.kind, ErrorKind::Name(NameError::TooLong(..))),
        "{error}"
    );
    one(&format!("theory: a\nnamespace: b\n  r<..{dotted}>\n"));
    let name = "n".repeat(MAX_RELATION_BYTES);
    one(&format!("namespace: t\n  {name}<@identity>"));
    let error = only(&format!("namespace: t\n  {name}n<@identity>"));
    assert!(
        matches!(error.kind, ErrorKind::Name(NameError::TooLong(..))),
        "{error}"
    );
}

/// A rewrite nesting `levels` levels: `this`, then a `! this` for each
/// level past the first.
fn chain(levels: usize) -> String {
    format!("this{}", " ! this".repeat(levels - 1))
}

/// `this` grouped in `groups` parentheses.
fn grouped(groups: usize) -> String {
    format!("{}this{}", "(".repeat(groups), ")".repeat(groups))
}

fn depth_case(rewrite: &str) -> String {
    format!("namespace: t\n  r<@identity>: {rewrite}")
}

/// Parsing recurses once for each level of grouping, so a depth set past
/// the default needs a larger stack than a test thread's.
fn with_stack(test: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(test)
        .expect("a thread")
        .join()
        .expect("the test passes");
}

#[test]
fn the_depth_limit_holds_for_levels_and_for_parentheses() {
    with_stack(depth_limits);
}

fn depth_limits() {
    const AT: usize = "  r<@identity>: ".len();
    for settings in [
        Settings::default(),
        Settings {
            depth: 7,
            ..Settings::default()
        },
        Settings {
            depth: 3 * DEFAULT_REWRITE_DEPTH,
            ..Settings::default()
        },
    ] {
        let depth = settings.depth;
        let parse = |text: &str| tdl::parse_with(text, settings);
        parse(&depth_case(&chain(depth))).expect("a rewrite at the limit");
        let errors = parse(&depth_case(&chain(depth + 1))).expect_err("past the limit");
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            matches!(
                errors[0].kind,
                ErrorKind::Namespace(NamespaceError::TooDeep(limit)) if limit == depth
            ),
            "{}",
            errors[0]
        );
        // The `!` that goes past it.
        assert_eq!(errors[0].place, place(2, AT + 5 + 7 * (depth - 1) + 1));
        parse(&depth_case(&grouped(depth))).expect("parentheses at the limit");
        let errors = parse(&depth_case(&grouped(depth + 1))).expect_err("past the limit");
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            matches!(errors[0].kind, ErrorKind::TooManyParentheses(limit) if limit == depth),
            "{}",
            errors[0]
        );
        assert_eq!(errors[0].place, place(2, AT + depth + 1));
    }
    assert_eq!(Settings::default().depth, DEFAULT_REWRITE_DEPTH);
    // Closed groups leave the nesting depth.
    let groups = vec!["(this)"; DEFAULT_REWRITE_DEPTH + 1].join(" | ");
    one(&depth_case(&groups));
    // Levels count runs, not operators.
    let wide = vec!["this"; 10 * DEFAULT_REWRITE_DEPTH].join(" | ");
    one(&depth_case(&wide));
}

#[test]
fn the_depth_limit_counts_every_kind_of_level() {
    // Alternating runs, each nesting the last in parentheses: a level for
    // each run, a group for each pair of parentheses.
    let nested = |levels: usize| {
        let mut rewrite = "this".to_owned();
        for level in 1..levels {
            let operator = if level % 2 == 0 { "|" } else { "&" };
            rewrite = format!("this {operator} ({rewrite})");
        }
        rewrite
    };
    one(&depth_case(&nested(DEFAULT_REWRITE_DEPTH)));
    // Past the limit, the levels go past it a group before the
    // parentheses do.
    let error = only(&depth_case(&nested(DEFAULT_REWRITE_DEPTH + 1)));
    assert!(
        matches!(error.kind, ErrorKind::Namespace(NamespaceError::TooDeep(_))),
        "{error}"
    );
    let settings = Settings {
        depth: DEFAULT_REWRITE_DEPTH + 10,
        ..Settings::default()
    };
    let errors = tdl::parse_with(&depth_case(&nested(DEFAULT_REWRITE_DEPTH + 2)), settings)
        .map(|_| ())
        .err();
    assert!(errors.is_none(), "{errors:?}");
}

#[test]
fn there_is_no_cap_on_relations_or_rewrite_length() {
    let relations: String = (0..2000).map(|i| format!("  r{i}<@identity>\n")).collect();
    let document = one(&format!("namespace: t\n{relations}"));
    assert_eq!(document.relations.relations().count(), 2000);
    let long = vec!["this"; 5000].join(" | ");
    one(&depth_case(&long));
}

// Printing.

#[test]
fn the_printer_writes_the_canonical_form() {
    let documents = parsed(DRIVE);
    let mut reversed = parsed(DRIVE);
    reversed.reverse();
    let expected = "theory: drive

namespace: file
  auditor<@identity>: this & viewer
  banned<@identity>
  editor<..group#member, @identity>: this | owner
  owner<@identity>
  parent<@identity, folder, folder#owner>
  viewer<..group#member, @identity>: (this | editor | (parent, viewer)) ! banned
namespace: folder
  banned<..group#member, @identity>
  owner<@identity>
  parent<folder>
  viewer<..group#member, @identity>: (this | (parent, viewer)) ! banned
";
    assert_eq!(print(&documents), Ok(expected.to_owned()));
    assert_eq!(print(&reversed), Ok(expected.to_owned()));
    assert_eq!(
        print(&parsed("namespace: user")),
        Ok("namespace: user\n".to_owned())
    );
}

#[test]
fn every_fixture_is_in_canonical_form() {
    for text in FIXTURES {
        assert_eq!(print(&parsed(text)), Ok(text.to_owned()));
    }
}

#[test]
fn undotted_namespaces_print_first_then_each_theory() {
    let documents = parsed(
        "namespace: b
namespace: a
theory: y
namespace: a
theory: x
namespace: b
         namespace: c.d
namespace: e.f
namespace: x.g
theory: c
namespace: d
",
    );
    assert_eq!(
        names(&documents),
        ["b", "a", "y.a", "x.b", "x.c.d", "x.e.f", "x.x.g", "c.d"]
    );
    let expected = "namespace: a
namespace: b

theory: c

namespace: d

theory: x

namespace: b
namespace: c.d
namespace: e.f
namespace: x.g

theory: y

namespace: a
";
    let printed = print(&documents).expect("printable namespaces");
    assert_eq!(printed, expected);
    let again = parsed(&printed);
    assert!(same(&documents, &again), "{printed}");
    // A theory alone, and plain namespaces alone, have no blank line
    // before them.
    assert_eq!(
        print(&parsed("theory: x\nnamespace: a\nnamespace: b\n")),
        Ok("theory: x\n\nnamespace: a\nnamespace: b\n".to_owned())
    );
    assert_eq!(
        print(&parsed("namespace: b\nnamespace: a\n")),
        Ok("namespace: a\nnamespace: b\n".to_owned())
    );
}

#[test]
fn a_constraint_outside_its_namespaces_theory_prints_with_dots() {
    let documents = parsed(
        "namespace: file
  owner<@identity, drive.folder#owner>
theory: drive
namespace: folder
  owner<@identity, ..file#owner, ..io.pipe, folder, ..sungma.group#member, x.y#r>
namespace: x.y
  r<..drive, ..drive.folder>
",
    );
    assert_eq!(
        constraints_of(&documents[1], "owner"),
        [
            "@identity",
            "drive.folder",
            "drive.x.y#r",
            "file#owner",
            "io.pipe",
            "sungma.group#member",
        ]
    );
    assert_eq!(
        constraints_of(&documents[2], "r"),
        ["drive", "drive.folder"]
    );
    let expected = "namespace: file
  owner<@identity, drive.folder#owner>

theory: drive

namespace: folder
  owner<..file#owner, ..io.pipe, ..sungma.group#member, @identity, folder, x.y#r>
namespace: x.y
  r<..drive, folder>
";
    let printed = print(&documents).expect("printable namespaces");
    assert_eq!(printed, expected);
    let again = parsed(&printed);
    assert!(same(&documents, &again), "{printed}");
    assert_eq!(print(&again), Ok(printed));
}

#[test]
fn constraints_print_sorted_with_the_prefix_taken_off() {
    let documents = parsed(
        "theory: t\nnamespace: a\n  r<..t.t.b, @identity, c#s, ..t.identity, ..u.v.w>: this | s\n  s<@identity>\n",
    );
    assert_eq!(
        constraints_of(&documents[0], "r"),
        ["@identity", "t.c#s", "t.identity", "t.t.b", "u.v.w"]
    );
    let printed = print(&documents).expect("a printable namespace");
    assert_eq!(
        printed,
        "theory: t\n\nnamespace: a\n  r<..u.v.w, @identity, c#s, identity, t.b>: this | s\n  s<@identity>\n"
    );
    let again = parsed(&printed);
    assert!(same(&documents, &again));
    // Outside a theory, names print as they are, with no dots.
    let documents = parsed("namespace: a\n  r<b, ..x.y, b#r>\n");
    assert_eq!(
        print(&documents),
        Ok("namespace: a\n  r<b, b#r, x.y>\n".to_owned())
    );
}

#[test]
fn the_printer_refuses_what_no_document_declares() {
    assert_eq!(print(&[]), Err(PrintError::NoNamespace));
    let mut twice = parsed(DRIVE);
    twice.extend(parsed(DRIVE));
    assert_eq!(
        print(&twice),
        Err(PrintError::DuplicateNamespace("drive.file".to_owned()))
    );
    let with = |namespace: &str, relations: Vec<(&str, Rewrite<&str>, Vec<NamedConstraint>)>| {
        let relations = relations
            .into_iter()
            .map(|(name, rewrite, constraints)| {
                (
                    relation(name),
                    rewrite.map(&mut |name| relation(name)),
                    constraints,
                )
            })
            .collect();
        NamespaceDocument {
            namespace: namespace_name(namespace),
            relations: Namespace::new(relations).expect("a valid namespace"),
        }
    };
    let identities = || vec![Constraint::Identity];
    let one_operand = with("t", vec![("r", Union(nonempty![This]), identities())]);
    assert_eq!(print(&[one_operand]), Err(PrintError::OneOperand));
    let one_operand = with(
        "t",
        vec![("r", Intersection(nonempty![This]), identities())],
    );
    assert_eq!(print(&[one_operand]), Err(PrintError::OneOperand));
    let excluded = with(
        "t",
        vec![("r", exclusion(This, Union(nonempty![This])), identities())],
    );
    assert_eq!(print(&[excluded]), Err(PrintError::OneOperand));
    for namespace in ["a..b", "t.", "1t", "a b", "a:b"] {
        let bad = with(namespace, vec![]);
        assert!(
            matches!(print(&[bad]), Err(PrintError::Name(_))),
            "{namespace}"
        );
    }
    for (name, rewrite) in [
        ("this", This),
        ("This", This),
        ("r-s", This),
        ("r:s", This),
        ("r", exclusion(This, Computed("THIS"))),
        (
            "r",
            exclusion(
                This,
                FactTo {
                    factset: "theory",
                    computed: "r",
                },
            ),
        ),
        (
            "r",
            exclusion(
                This,
                FactTo {
                    factset: "r",
                    computed: "a b",
                },
            ),
        ),
    ] {
        let mut relations = vec![(name, rewrite, identities())];
        for declared in ["THIS", "theory", "a b"] {
            relations.push((declared, This, identities()));
        }
        relations.dedup_by_key(|(name, ..)| *name);
        let bad = with("t", relations);
        assert!(matches!(print(&[bad]), Err(PrintError::Name(_))), "{name}");
    }
    for constraint in [
        Constraint::Resource(namespace_name("a b")),
        Constraint::Subjectset(namespace_name("t"), relation("this")),
        Constraint::Subjectset(namespace_name("t"), relation("a b")),
        Constraint::Subjectset(namespace_name("a.1"), relation("member")),
        Constraint::Resource(namespace_name("a:b")),
        Constraint::Subjectset(namespace_name("t"), relation("a:b")),
    ] {
        let bad = with("t", vec![("r", This, vec![constraint.clone()])]);
        assert!(
            matches!(print(&[bad]), Err(PrintError::Name(_))),
            "{constraint:?}"
        );
    }
}

#[test]
fn the_printer_refuses_a_reference_the_parser_would() {
    let with = |namespace: &str, relations: Vec<(&str, Rewrite<&str>, Vec<NamedConstraint>)>| {
        let relations = relations
            .into_iter()
            .map(|(name, rewrite, constraints)| {
                (
                    relation(name),
                    rewrite.map(&mut |name| relation(name)),
                    constraints,
                )
            })
            .collect();
        NamespaceDocument {
            namespace: namespace_name(namespace),
            relations: Namespace::new(relations).expect("a valid namespace"),
        }
    };
    let subjectset = |namespace: &str, name: &str| {
        Constraint::Subjectset(namespace_name(namespace), relation(name))
    };
    let b = || with("b", vec![("m", This, vec![Constraint::Identity])]);
    // A constraint naming a relation another printed namespace lacks.
    let a = with("a", vec![("r", This, vec![subjectset("b", "x")])]);
    let documents = [a, b()];
    let printed = print(&documents);
    assert!(
        matches!(
            &printed,
            Err(PrintError::Reference { namespace, error: NamespaceError::UndeclaredConstraint { .. } })
                if namespace == "a"
        ),
        "{printed:?}"
    );
    // One naming a relation its own namespace lacks.
    let s = with("s", vec![("r", This, vec![subjectset("s", "x")])]);
    assert!(
        matches!(
            print(&[s]),
            Err(PrintError::Reference {
                error: NamespaceError::UndeclaredConstraint { .. },
                ..
            })
        ),
        "a self-reference"
    );
    // A fact-to-subjectset following a relation another lacks.
    let a = with(
        "a",
        vec![
            ("p", This, vec![Constraint::Resource(namespace_name("b"))]),
            (
                "v",
                FactTo {
                    factset: "p",
                    computed: "x",
                },
                vec![],
            ),
        ],
    );
    assert!(
        matches!(
            print(&[a, b()]),
            Err(PrintError::Reference {
                error: NamespaceError::UndeclaredFollowed { .. },
                ..
            })
        ),
        "a followed relation"
    );
    // What the parser accepts prints: references that hold, and those to a
    // namespace outside the printed set.
    let a = with(
        "a",
        vec![
            (
                "r",
                This,
                vec![subjectset("b", "m"), subjectset("elsewhere", "x")],
            ),
            (
                "p",
                This,
                vec![Constraint::Resource(namespace_name("elsewhere"))],
            ),
            (
                "v",
                FactTo {
                    factset: "p",
                    computed: "x",
                },
                vec![],
            ),
        ],
    );
    let documents = [a, b()];
    let printed = print(&documents).expect("references that hold");
    assert!(same(&parsed(&printed), &documents));
}

#[test]
fn the_printer_refuses_a_rewrite_the_parser_would() {
    let at_limit = parsed(&depth_case(&chain(DEFAULT_REWRITE_DEPTH)));
    let printed = print(&at_limit).expect("a rewrite at the limit");
    assert!(same(&parsed(&printed), &at_limit));
    // Grouped on the right, each level past the first adds parentheses.
    let mut right = This;
    for _ in 1..DEFAULT_REWRITE_DEPTH {
        right = exclusion(This, right);
    }
    let with = |rewrite: Rewrite<RelationName>| NamespaceDocument {
        namespace: namespace_name("t"),
        relations: Namespace::with_depth(
            vec![(relation("r"), rewrite, vec![Constraint::Identity])],
            2 * DEFAULT_REWRITE_DEPTH,
        )
        .expect("a valid namespace"),
    };
    let printed = print(&[with(right.clone())]).expect("a rewrite at the limit");
    parsed(&printed);
    assert_eq!(
        print(&[with(exclusion(This, right))]),
        Err(PrintError::TooDeep)
    );
    let past = tdl::parse_with(
        &depth_case(&chain(DEFAULT_REWRITE_DEPTH + 1)),
        Settings {
            depth: DEFAULT_REWRITE_DEPTH + 1,
            ..Settings::default()
        },
    )
    .expect("a rewrite at a deeper limit");
    assert_eq!(print(&past), Err(PrintError::TooDeep));
}

#[test]
fn a_rewrite_built_far_past_the_depth_limit_is_refused_without_a_crash() {
    // Building, checking and dropping the tree recurses through it, so
    // they take the larger stack; printing takes a thread's usual one.
    with_stack(|| {
        let mut rewrite = This;
        for _ in 1..100_000 {
            rewrite = exclusion(rewrite, This);
        }
        let document = NamespaceDocument {
            namespace: namespace_name("t"),
            relations: Namespace::with_depth(
                vec![(relation("r"), rewrite, vec![Constraint::Identity])],
                usize::MAX,
            )
            .expect("a valid namespace"),
        };
        let printed = std::thread::scope(|scope| {
            scope
                .spawn(|| print(std::slice::from_ref(&document)))
                .join()
                .expect("printing doesn't crash")
        });
        assert_eq!(printed, Err(PrintError::TooDeep));
    });
}

/// A namespace `t` printed as exactly `bytes`: relations sorted by name,
/// each taking identities, the last one's name making up the rest.
fn printed_at(bytes: usize) -> String {
    const HEADER: &str = "namespace: t\n";
    // `  r0000000<@identity>\n`, and `  z<@identity>\n` with a longer name.
    const EACH: usize = 22;
    const LAST: usize = 15;
    let count = (bytes - HEADER.len() - LAST) / EACH;
    let rest = bytes - HEADER.len() - EACH * count - LAST;
    assert!(rest < MAX_RELATION_BYTES);
    let relations: String = (0..count)
        .map(|i| format!("  r{i:07}<@identity>\n"))
        .collect();
    let text = format!("{HEADER}{relations}  z{}<@identity>\n", "z".repeat(rest));
    assert_eq!(text.len(), bytes);
    text
}

#[test]
fn the_printer_has_no_size_limit() {
    // Past 2 MiB, printed, and read back as a stored version is.
    let text = printed_at(LARGE + 1);
    let documents = parsed(&text);
    let printed = print(&documents).expect("no size limit");
    assert_eq!(printed, text);
    let again = tdl::parse_one(&printed).expect("no size limit");
    assert!(same(&documents, &[again]));
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
/// leaf beside each step. A run of unions in unions, or anything on the
/// right of `!`, nests parentheses as deep as the printer writes them: two
/// short of the depth limit, as only an operator below the root is grouped.
fn deep(at: usize) -> BoxedStrategy<Rewrite<RelationName>> {
    let levels = prop_oneof![Just(DEFAULT_REWRITE_DEPTH - 1), 0..DEFAULT_REWRITE_DEPTH];
    (leaf(at), levels)
        .prop_flat_map(move |(first, levels)| {
            vec((0..5u8, leaf(at)), levels).prop_map(move |spine| {
                spine
                    .into_iter()
                    .fold(first.clone(), |inner, (kind, beside)| match kind {
                        0 => Union(nonempty![inner, beside]),
                        1 => Intersection(nonempty![inner, beside]),
                        2 => exclusion(inner, beside),
                        3 => Union(nonempty![beside, inner]),
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

/// Constraints of every kind: identities, resources of a plain namespace,
/// of one in the `drive` theory and of one in `io`, subjectsets of another
/// namespace in and outside a theory, and of the namespace itself, `SELF`.
/// Whatever theory the namespace is in, some are in it and some outside,
/// so the printer writes some with `..`. `..` outside a theory changes
/// nothing.
const KINDS: [&str; 10] = [
    "@identity",
    "folder",
    "drive.file",
    "drive.x.file",
    "..io.folder",
    "drive.group#member",
    "group#member",
    "t.group#member",
    "SELF#r1",
    "SELF#r3",
];

/// One to four distinct constraints of namespace `own`, in any order.
fn constraints(own: &'static str) -> impl Strategy<Value = Vec<NamedConstraint>> {
    proptest::sample::subsequence(KINDS.to_vec(), 1..5)
        .prop_shuffle()
        .prop_map(move |kinds| {
            kinds
                .into_iter()
                .map(|kind| tdl::constraint(&kind.replace("SELF", own)).expect("a constraint"))
                .collect()
        })
}

type Generated = Vec<(RelationName, Rewrite<RelationName>, Vec<NamedConstraint>)>;

fn relations(own: &'static str) -> impl Strategy<Value = Generated> {
    let declared = move |at| {
        (tree(at), constraints(own)).prop_map(|(tree, constraints)| {
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

/// Whether a rewrite reads `this`.
fn reads_this<R>(rewrite: &Rewrite<R>) -> bool {
    match rewrite {
        This => true,
        Computed(_) | FactTo { .. } => false,
        Union(operands) | Intersection(operands) => operands.iter().any(reads_this),
        Exclusion(base, excluded) => reads_this(base) || reads_this(excluded),
    }
}

/// Namespaces under no theory, one, or several, each named once.
fn theories() -> impl Strategy<Value = Vec<&'static str>> {
    let pool =
        |names: &[&'static str]| proptest::sample::subsequence(names.to_vec(), 1..=names.len());
    prop_oneof![
        pool(&["t", "u"]),
        pool(&["drive.t", "drive.u", "drive.x.y"]),
        pool(&["t", "drive.t", "drive.x.y", "io.t", "io.u", "sungma.root"]),
    ]
}

fn namespaces() -> impl Strategy<Value = Vec<(&'static str, Generated)>> {
    theories().prop_flat_map(|names| {
        names
            .into_iter()
            .map(|own| relations(own).prop_map(move |relations| (own, relations)))
            .collect::<Vec<_>>()
    })
}

const TOKENS: [&str; 12] = [
    "this", "r0", "r1", "r2", "(", ")", ",", "|", "&", "!", " ", "\n",
];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Printing namespaces and parsing them back gives equal namespaces,
    /// and printing those gives the same text.
    #[test]
    fn printed_namespaces_parse_back_equal(namespaces in namespaces()) {
        let documents: Vec<_> = namespaces
            .into_iter()
            .map(|(name, relations)| NamespaceDocument {
                namespace: namespace_name(name),
                relations: Namespace::new(relations).expect("a valid namespace"),
            })
            .collect();
        let text = print(&documents).map_err(|error| TestCaseError::fail(error.to_string()))?;
        let again = tdl::parse(&text)
            .map_err(|errors| TestCaseError::fail(format!("{text}\n{errors:?}")))?;
        prop_assert!(same(&documents, &again), "{}", text);
        prop_assert_eq!(print(&again), Ok(text));
    }
}

proptest! {
    /// Rewrite text built from the grammar's tokens is refused, or parses
    /// to a tree that prints and parses back equal.
    #[test]
    fn rewrite_text_parses_prints_and_parses_alike(
        tokens in vec(proptest::sample::select(TOKENS.to_vec()), 1..40),
    ) {
        let expression = tokens.concat();
        let text = format!(
            "namespace: t\n  r0<@identity>\n  r1<@identity>\n  r2<@identity>\n{}{expression}",
            signature(&expression)
        );
        if let Ok(documents) = tdl::parse(&text) {
            let printed = print(&documents)
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            let again = parsed(&printed);
            prop_assert!(same(&again, &documents));
            prop_assert_eq!(print(&again), Ok(printed));
        }
    }

    /// The parser never panics, whatever text it is given.
    #[test]
    fn any_text_is_parsed_or_refused(text in any::<String>()) {
        let _ = tdl::parse(&text);
    }

    /// The parser never panics on near-TDL text, and what it accepts prints
    /// and parses back.
    #[test]
    fn any_near_tdl_text_is_parsed_or_refused(
        text in r"(theory|namespace|this|@identity|\.\.|[:<>.,()|&!/ \n#rt0-9])*",
    ) {
        if let Ok(documents) = tdl::parse(&text) {
            let printed = print(&documents)
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            let again = parsed(&printed);
            prop_assert!(same(&again, &documents));
            prop_assert_eq!(print(&again), Ok(printed));
        }
    }
}

// Hostile input.

/// Generous for a linear pass in a debug build, and far short of what
/// quadratic work on these documents takes.
const BOUND: Duration = Duration::from_secs(60);

#[test]
fn the_longest_chain_of_computed_subjectsets_is_accepted() {
    let mut relations = String::from("namespace: t\n");
    let mut count = 0;
    loop {
        let line = format!("  r{count}: r{}\n", count + 1);
        // Room for the last relation, which takes identities.
        if relations.len() + line.len() + 32 > LARGE {
            break;
        }
        relations.push_str(&line);
        count += 1;
    }
    relations.push_str(&format!("  r{count}<@identity>\n"));
    assert!(relations.len() <= LARGE);
    assert!(count >= 100_000, "{count} computed subjectsets");
    let start = Instant::now();
    let document = one(&relations);
    assert!(start.elapsed() < BOUND);
    assert_eq!(document.relations.relations().count(), count + 1);
}

#[test]
fn the_largest_set_of_relations_reaching_each_other_is_one_cycle() {
    let line =
        |i: usize, count: usize| format!("  r{i}: r{} | r{}\n", (i + 1) % count, (i + 7) % count);
    // As many as fit, each line written for that many.
    let mut count = 1000;
    loop {
        let size: usize = 13
            + (0..count + 1000)
                .map(|i| line(i, count + 1000).len())
                .sum::<usize>();
        if size > LARGE {
            break;
        }
        count += 1000;
    }
    let ring: String = (0..count).map(|i| line(i, count)).collect();
    let text = format!("namespace: t\n{ring}");
    assert!(text.len() <= LARGE);
    let start = Instant::now();
    let error = only(&text);
    assert!(start.elapsed() < BOUND);
    let ErrorKind::Namespace(NamespaceError::RewriteCycle { path, .. }) = error.kind else {
        panic!("expected a cycle, got {error}");
    };
    assert_eq!(path.first(), path.last());
    assert_eq!(error.relation.as_deref(), Some("r0"));
}

#[test]
fn a_long_chain_of_operators_is_refused_without_a_crash() {
    let chain = vec!["b"; 900_000].join("!");
    let text = format!("namespace: t\n  b<@identity>\n  a: {chain}");
    assert!(text.len() <= LARGE);
    let start = Instant::now();
    let error = only(&text);
    assert!(start.elapsed() < BOUND);
    assert!(
        matches!(error.kind, ErrorKind::Namespace(NamespaceError::TooDeep(_))),
        "{error}"
    );
    assert_eq!(error.place, place(3, 5 + 2 * DEFAULT_REWRITE_DEPTH));
    let groups = 1_000_000;
    let error = only(&format!(
        "namespace: t\n  a<@identity>: {}",
        grouped(groups)
    ));
    assert!(
        matches!(error.kind, ErrorKind::TooManyParentheses(_)),
        "{error}"
    );
    let opens = "(".repeat(LARGE - 100);
    let error = only(&format!("namespace: t\n  a<@identity>: {opens}"));
    assert!(
        matches!(error.kind, ErrorKind::TooManyParentheses(_)),
        "{error}"
    );
}

#[test]
fn a_namespace_built_far_larger_than_a_document_is_checked_without_a_crash() {
    let count = 300_000;
    let mut relations: Vec<_> = (0..count)
        .map(|i| {
            (
                relation(&format!("r{i}")),
                name(&format!("r{}", i + 1)),
                vec![],
            )
        })
        .collect();
    relations.push((
        relation(&format!("r{count}")),
        This,
        vec![Constraint::Identity],
    ));
    let start = Instant::now();
    let chain = Namespace::new(relations).expect("a chain is valid");
    let document = NamespaceDocument {
        namespace: namespace_name("t"),
        relations: chain,
    };
    let printed = print(&[document]).expect("no size limit");
    assert!(printed.len() > LARGE);
    let ring: Vec<_> = (0..count)
        .map(|i| {
            (
                relation(&format!("r{i}")),
                name(&format!("r{}", (i + 1) % count)),
                vec![],
            )
        })
        .collect::<Vec<(RelationName, Rewrite<RelationName>, Vec<NamedConstraint>)>>();
    let problems = Namespace::new(ring).expect_err("a ring is a cycle");
    assert_eq!(problems.len(), 1);
    assert!(start.elapsed() < BOUND);
}
