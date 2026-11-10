//! The TDL printer: namespaces written as the document [`parse`](super::parse)
//! reads back as equal namespaces, the same namespaces always as the same
//! text.

use std::fmt::Write;

use sungma::{
    name::{NamespaceName, RelationName},
    namespace::Constraint,
    rewrite::Rewrite,
};

use super::{IDENTITY, Named, NamedConstraint, OUTSIDE};

/// Writes namespaces as a document that [`parse`](super::parse) reads back
/// as equal namespaces, as `docs/specs/theory-documents.md` says: namespaces
/// without a dot first, with no `theory:` line, then each theory, the first
/// segment of its namespaces' names, with its namespaces under it and the
/// theory's name and dot taken off; namespaces, relations and constraints
/// sorted, comparing bytes; one relation to a line, indented two spaces;
/// `: this` left out; only the parentheses the grammar needs; lines ending
/// in `\n`. A blank line comes after each `theory:` line, and before each
/// one after the start; namespaces follow one another with none between.
///
/// A constraint naming a namespace outside its own namespace's theory,
/// including one without a dot, is written in full after `..`.
///
/// The printer only prints: it checks nothing and never refuses. No
/// namespaces print as an empty document, a namespace given twice prints
/// twice, a union or intersection of one operand prints its operand, and a
/// name prints as it is, whether a document could declare it or not. Only
/// namespaces the parser accepts print as a document it reads back.
///
/// The printed document has no size limit, and a rewrite of any depth
/// prints without recursing through it.
pub fn print(namespaces: &[(&NamespaceName, &Named)]) -> String {
    let mut sorted: Vec<_> = namespaces
        .iter()
        .map(|&(name, relations)| {
            let (theory, written) = match name.as_str().split_once('.') {
                Some((theory, rest)) => (Some(theory), rest),
                None => (None, name.as_str()),
            };
            (theory, name, written, relations)
        })
        .collect();
    // `None` sorts first, so namespaces without a dot come before every
    // theory.
    sorted.sort_by(|(one, a, ..), (other, b, ..)| (one, a).cmp(&(other, b)));
    let mut text = String::new();
    let mut section = None;
    for (theory, _, written, relations) in sorted {
        if let Some(theory) = theory
            && section != Some(theory)
        {
            if !text.is_empty() {
                text.push('\n');
            }
            let _ = writeln!(text, "theory: {theory}\n");
            section = Some(theory);
        }
        let _ = writeln!(text, "namespace: {written}");
        namespace(&mut text, theory, relations);
    }
    text
}

/// Writes the relations of a namespace in `theory`, sorted by name.
fn namespace(text: &mut String, theory: Option<&str>, relations: &Named) {
    let mut sorted: Vec<_> = relations.relations().collect();
    sorted.sort_by_key(|(relation, _)| *relation);
    for (relation, rewrite) in sorted {
        let mut constraints: Vec<_> = relations
            .constraints(relation)
            .iter()
            .map(|constraint| written_constraint(constraint, theory))
            .collect();
        constraints.sort();
        let mut expression = String::new();
        self::expression(&mut expression, rewrite);
        let _ = match (constraints.is_empty(), expression == "this") {
            (true, _) => writeln!(text, "  {relation}: {expression}"),
            (false, true) => writeln!(text, "  {relation}<{}>", constraints.join(", ")),
            (false, false) => writeln!(
                text,
                "  {relation}<{}>: {expression}",
                constraints.join(", ")
            ),
        };
    }
}

/// A constraint as written under `theory`: the theory's name and dot taken
/// off a namespace in it, and `..` before one outside it.
fn written_constraint(constraint: &NamedConstraint, theory: Option<&str>) -> String {
    let named = |name: &NamespaceName| {
        let name = name.as_str();
        match theory {
            None => name.to_owned(),
            Some(theory) => match name
                .strip_prefix(theory)
                .and_then(|rest| rest.strip_prefix('.'))
            {
                Some(rest) => rest.to_owned(),
                None => format!("{OUTSIDE}{name}"),
            },
        }
    };
    match constraint {
        Constraint::Identity => IDENTITY.to_owned(),
        Constraint::Resource(name) => named(name),
        Constraint::Subjectset(name, relation) => format!("{}#{relation}", named(name)),
    }
}

/// What is left to write of a rewrite: a node, in parentheses or not, or
/// text that follows one. A node is pushed as what it prints as, so its
/// parentheses are decided by that.
enum Step<'r> {
    Node(&'r Rewrite<RelationName>, bool),
    Text(&'static str),
}

/// Writes `rewrite`, walking it with a stack of its own rather than
/// recursion, so a tree of any depth prints without overflowing the stack.
/// Grouping parentheses are written only where the grammar needs them: a
/// union inside a union, a union or intersection inside an intersection or
/// on the left of `!`, and any operator on the right of `!`.
fn expression(text: &mut String, rewrite: &Rewrite<RelationName>) {
    let mut steps = vec![Step::Node(alone(rewrite), false)];
    while let Some(step) = steps.pop() {
        let (rewrite, parenthesize) = match step {
            Step::Text(written) => {
                text.push_str(written);
                continue;
            }
            Step::Node(rewrite, parenthesize) => (rewrite, parenthesize),
        };
        if parenthesize {
            text.push('(');
            steps.push(Step::Text(")"));
        }
        match rewrite {
            Rewrite::This => text.push_str("this"),
            Rewrite::Computed(relation) => text.push_str(relation.as_str()),
            Rewrite::FactTo { factset, computed } => {
                let _ = write!(text, "({factset}, {computed})");
            }
            Rewrite::Union(operands) => {
                operator(&mut steps, operands.iter(), " | ", |operand| {
                    matches!(operand, Rewrite::Union(_))
                });
            }
            Rewrite::Intersection(operands) => {
                operator(&mut steps, operands.iter(), " & ", |operand| {
                    matches!(operand, Rewrite::Union(_) | Rewrite::Intersection(_))
                });
            }
            Rewrite::Exclusion(base, excluded) => {
                // Pushed last first, so the base is written first.
                let excluded = alone(excluded);
                steps.push(Step::Node(
                    excluded,
                    matches!(
                        excluded,
                        Rewrite::Union(_) | Rewrite::Intersection(_) | Rewrite::Exclusion(..)
                    ),
                ));
                steps.push(Step::Text(" ! "));
                let base = alone(base);
                steps.push(Step::Node(
                    base,
                    matches!(base, Rewrite::Union(_) | Rewrite::Intersection(_)),
                ));
            }
        }
    }
}

/// Pushes `operands` joined by `symbol`, last first, so the first is
/// written first.
fn operator<'r>(
    steps: &mut Vec<Step<'r>>,
    operands: impl DoubleEndedIterator<Item = &'r Rewrite<RelationName>>,
    symbol: &'static str,
    needs_parentheses: fn(&Rewrite<RelationName>) -> bool,
) {
    for (i, operand) in operands.rev().enumerate() {
        if i > 0 {
            steps.push(Step::Text(symbol));
        }
        let operand = alone(operand);
        steps.push(Step::Node(operand, needs_parentheses(operand)));
    }
}

/// What `rewrite` prints as: past any union or intersection of one operand,
/// which prints as its operand.
fn alone(mut rewrite: &Rewrite<RelationName>) -> &Rewrite<RelationName> {
    while let Rewrite::Union(operands) | Rewrite::Intersection(operands) = rewrite
        && let [operand] = &operands[..]
    {
        rewrite = operand;
    }
    rewrite
}
