//! The TDL printer: namespaces written as the document [`parse`](super::parse)
//! reads back as equal namespaces, the same namespaces always as the same
//! text.

use std::fmt::Write;

use thiserror::Error;

use sungma::{
    name::{NameError, NamespaceName, RelationName, quote},
    namespace::Constraint,
    rewrite::Rewrite,
};

use super::{IDENTITY, MAX_DOCUMENT_BYTES, Named, NamedConstraint, namespace_name};

/// Why [`print()`] refused namespaces: no document can declare them.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PrintError {
    /// A name made with `new_unchecked` that breaks the grammar, such as
    /// `this`.
    #[error(transparent)]
    Name(#[from] NameError),
    #[error("a union or intersection has one operand")]
    OneOperand,
    #[error("a document declares at least one namespace")]
    NoNamespace,
    #[error("namespace '{0}' is given twice")]
    DuplicateNamespace(String),
    /// A namespace in a theory with a constraint naming a namespace without
    /// a dot, which its theory's name would prefix.
    #[error(
        "namespace '{namespace}' is in a theory, so it can't name namespace '{constraint}', which has no dot"
    )]
    Undotted {
        namespace: String,
        constraint: String,
    },
    #[error("the printed document would be more than {MAX_DOCUMENT_BYTES} bytes")]
    TooLarge,
}

/// Writes namespaces as a document that [`parse`](super::parse) reads back
/// as equal namespaces, as `docs/specs/theory-documents.md` says: namespaces
/// without a dot first, with no `theory:` line, then each theory, the first
/// segment of its namespaces' names, with its namespaces under it and the
/// prefix taken off; namespaces, relations and constraints sorted, comparing
/// bytes; one relation to a line, indented two spaces; `: this` left out;
/// only the parentheses the grammar needs; lines ending in `\n`.
///
/// A namespace whose name still has a dot once its theory's name is taken
/// off is written in full, as is every dotted constraint outside its
/// namespace's theory, since a dotted name is never prefixed.
pub fn print(namespaces: &[(&NamespaceName, &Named)]) -> Result<String, PrintError> {
    if namespaces.is_empty() {
        return Err(PrintError::NoNamespace);
    }
    let mut sorted = Vec::new();
    for &(name, relations) in namespaces {
        namespace_name(name.as_str())?;
        let theory = name.as_str().split_once('.').map(|(theory, _)| theory);
        sorted.push((theory, name, relations));
    }
    // `None` sorts first, so namespaces without a dot come before every
    // theory.
    sorted.sort_by(|(one, a, _), (other, b, _)| (one, a).cmp(&(other, b)));
    let mut text = String::new();
    let mut section = None;
    let mut previous: Option<&NamespaceName> = None;
    for (theory, name, relations) in sorted {
        if previous == Some(name) {
            return Err(PrintError::DuplicateNamespace(quote(name.as_str())));
        }
        previous = Some(name);
        if let Some(theory) = theory
            && section != Some(theory)
        {
            if !text.is_empty() {
                text.push('\n');
            }
            let _ = writeln!(text, "theory: {theory}");
            section = Some(theory);
        }
        if !text.is_empty() {
            text.push('\n');
        }
        let _ = writeln!(text, "namespace: {}", written(name.as_str(), theory));
        namespace(&mut text, name, theory, relations)?;
        // Checked as it grows, so a caller's oversized input is refused
        // without being printed whole.
        if text.len() > MAX_DOCUMENT_BYTES {
            return Err(PrintError::TooLarge);
        }
    }
    Ok(text)
}

/// A namespace's name as written under `theory`: without the theory's
/// name and its dot, unless what's left still has a dot.
fn written<'a>(name: &'a str, theory: Option<&str>) -> &'a str {
    let rest = theory
        .and_then(|theory| name.strip_prefix(theory))
        .and_then(|rest| rest.strip_prefix('.'));
    match rest {
        Some(rest) if !rest.contains('.') => rest,
        _ => name,
    }
}

/// Writes the relations of namespace `name`, in `theory`, sorted by name.
fn namespace(
    text: &mut String,
    name: &NamespaceName,
    theory: Option<&str>,
    relations: &Named,
) -> Result<(), PrintError> {
    let mut sorted: Vec<_> = relations.relations().collect();
    sorted.sort_by_key(|(relation, _)| *relation);
    for (relation, rewrite) in sorted {
        let mut constraints = relations
            .constraints(relation)
            .iter()
            .map(|constraint| written_constraint(constraint, name, theory))
            .collect::<Result<Vec<_>, _>>()?;
        constraints.sort();
        let relation = printable(relation)?;
        let mut expression = String::new();
        self::expression(&mut expression, rewrite)?;
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
    Ok(())
}

/// A constraint of namespace `namespace` as written under `theory`, the
/// prefix taken off a namespace in that theory. One without a dot can't
/// be written under a theory.
fn written_constraint(
    constraint: &NamedConstraint,
    namespace: &NamespaceName,
    theory: Option<&str>,
) -> Result<String, PrintError> {
    let named = |name: &NamespaceName| {
        namespace_name(name.as_str())?;
        let name = name.as_str();
        if theory.is_some() && !name.contains('.') {
            return Err(PrintError::Undotted {
                namespace: quote(namespace.as_str()),
                constraint: quote(name),
            });
        }
        Ok(written(name, theory).to_owned())
    };
    Ok(match constraint {
        Constraint::Identity => IDENTITY.to_owned(),
        Constraint::Resource(name) => named(name)?,
        Constraint::Subjectset(name, relation) => {
            format!("{}#{}", named(name)?, printable(relation)?)
        }
    })
}

fn expression(text: &mut String, rewrite: &Rewrite<RelationName>) -> Result<(), PrintError> {
    match rewrite {
        Rewrite::This => text.push_str("this"),
        Rewrite::Computed(relation) => text.push_str(printable(relation)?),
        Rewrite::FactTo { factset, computed } => {
            let _ = write!(text, "({}, {})", printable(factset)?, printable(computed)?);
        }
        Rewrite::Union(operands) | Rewrite::Intersection(operands) if operands.len() < 2 => {
            return Err(PrintError::OneOperand);
        }
        Rewrite::Union(operands) => operator(text, operands, " | ", |operand| {
            matches!(operand, Rewrite::Union(_))
        })?,
        Rewrite::Intersection(operands) => operator(text, operands, " & ", |operand| {
            matches!(operand, Rewrite::Union(_) | Rewrite::Intersection(_))
        })?,
        Rewrite::Exclusion(base, excluded) => {
            grouped(
                text,
                base,
                matches!(**base, Rewrite::Union(_) | Rewrite::Intersection(_)),
            )?;
            text.push_str(" ! ");
            grouped(
                text,
                excluded,
                matches!(
                    **excluded,
                    Rewrite::Union(_) | Rewrite::Intersection(_) | Rewrite::Exclusion(..)
                ),
            )?;
        }
    }
    Ok(())
}

/// `name`, if a document can declare a relation by it.
fn printable(name: &RelationName) -> Result<&str, NameError> {
    name.as_str().parse::<RelationName>()?;
    Ok(name.as_str())
}

fn operator(
    text: &mut String,
    operands: &[Rewrite<RelationName>],
    symbol: &str,
    needs_parentheses: fn(&Rewrite<RelationName>) -> bool,
) -> Result<(), PrintError> {
    for (i, operand) in operands.iter().enumerate() {
        if i > 0 {
            text.push_str(symbol);
        }
        grouped(text, operand, needs_parentheses(operand))?;
    }
    Ok(())
}

fn grouped(
    text: &mut String,
    rewrite: &Rewrite<RelationName>,
    parenthesize: bool,
) -> Result<(), PrintError> {
    if parenthesize {
        text.push('(');
        expression(text, rewrite)?;
        text.push(')');
    } else {
        expression(text, rewrite)?;
    }
    Ok(())
}
