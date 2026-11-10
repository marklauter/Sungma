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

use super::{DEFAULT_REWRITE_DEPTH, IDENTITY, Named, NamedConstraint, OUTSIDE, namespace_name};

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
    /// A rewrite nesting deeper, in levels or in grouping parentheses, than
    /// [`parse`](super::parse) reads.
    #[error("a rewrite nests more than {DEFAULT_REWRITE_DEPTH} deep")]
    TooDeep,
}

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
/// The printed document has no size limit.
pub fn print(namespaces: &[(&NamespaceName, &Named)]) -> Result<String, PrintError> {
    if namespaces.is_empty() {
        return Err(PrintError::NoNamespace);
    }
    let mut sorted = Vec::new();
    for &(name, relations) in namespaces {
        namespace_name(name.as_str())?;
        let (theory, written) = match name.as_str().split_once('.') {
            Some((theory, rest)) => (Some(theory), rest),
            None => (None, name.as_str()),
        };
        sorted.push((theory, name, written, relations));
    }
    // `None` sorts first, so namespaces without a dot come before every
    // theory.
    sorted.sort_by(|(one, a, ..), (other, b, ..)| (one, a).cmp(&(other, b)));
    let mut text = String::new();
    let mut section = None;
    let mut previous: Option<&NamespaceName> = None;
    for (theory, name, written, relations) in sorted {
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
            let _ = writeln!(text, "theory: {theory}\n");
            section = Some(theory);
        }
        let _ = writeln!(text, "namespace: {written}");
        namespace(&mut text, theory, relations)?;
    }
    Ok(text)
}

/// Writes the relations of a namespace in `theory`, sorted by name.
fn namespace(text: &mut String, theory: Option<&str>, relations: &Named) -> Result<(), PrintError> {
    let mut sorted: Vec<_> = relations.relations().collect();
    sorted.sort_by_key(|(relation, _)| *relation);
    for (relation, rewrite) in sorted {
        let mut constraints = relations
            .constraints(relation)
            .iter()
            .map(|constraint| written_constraint(constraint, theory))
            .collect::<Result<Vec<_>, _>>()?;
        constraints.sort();
        let relation = printable(relation)?;
        let mut expression = String::new();
        self::expression(&mut expression, rewrite, 1)?;
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

/// A constraint as written under `theory`: the theory's name and dot taken
/// off a namespace in it, and `..` before one outside it.
fn written_constraint(
    constraint: &NamedConstraint,
    theory: Option<&str>,
) -> Result<String, PrintError> {
    let named = |name: &NamespaceName| -> Result<String, PrintError> {
        let name = name.as_str();
        namespace_name(name)?;
        Ok(match theory {
            None => name.to_owned(),
            Some(theory) => match name
                .strip_prefix(theory)
                .and_then(|rest| rest.strip_prefix('.'))
            {
                Some(rest) => rest.to_owned(),
                None => format!("{OUTSIDE}{name}"),
            },
        })
    };
    Ok(match constraint {
        Constraint::Identity => IDENTITY.to_owned(),
        Constraint::Resource(name) => named(name)?,
        Constraint::Subjectset(name, relation) => {
            format!("{}#{}", named(name)?, printable(relation)?)
        }
    })
}

/// Writes `rewrite`, `level` levels deep, the root being 1. A rewrite past
/// the default depth limit is refused before it's walked further, so one
/// built in code, however deep, can't overflow the stack. Grouping
/// parentheses are written only around an operand with an operator, so
/// they never nest deeper than the levels.
fn expression(
    text: &mut String,
    rewrite: &Rewrite<RelationName>,
    level: usize,
) -> Result<(), PrintError> {
    if level > DEFAULT_REWRITE_DEPTH {
        return Err(PrintError::TooDeep);
    }
    let next = level + 1;
    match rewrite {
        Rewrite::This => text.push_str("this"),
        Rewrite::Computed(relation) => text.push_str(printable(relation)?),
        Rewrite::FactTo { factset, computed } => {
            let _ = write!(text, "({}, {})", printable(factset)?, printable(computed)?);
        }
        Rewrite::Union(operands) | Rewrite::Intersection(operands) if operands.len() < 2 => {
            return Err(PrintError::OneOperand);
        }
        Rewrite::Union(operands) => operator(text, operands, " | ", next, |operand| {
            matches!(operand, Rewrite::Union(_))
        })?,
        Rewrite::Intersection(operands) => operator(text, operands, " & ", next, |operand| {
            matches!(operand, Rewrite::Union(_) | Rewrite::Intersection(_))
        })?,
        Rewrite::Exclusion(base, excluded) => {
            grouped(
                text,
                base,
                next,
                matches!(**base, Rewrite::Union(_) | Rewrite::Intersection(_)),
            )?;
            text.push_str(" ! ");
            grouped(
                text,
                excluded,
                next,
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
    level: usize,
    needs_parentheses: fn(&Rewrite<RelationName>) -> bool,
) -> Result<(), PrintError> {
    for (i, operand) in operands.iter().enumerate() {
        if i > 0 {
            text.push_str(symbol);
        }
        grouped(text, operand, level, needs_parentheses(operand))?;
    }
    Ok(())
}

fn grouped(
    text: &mut String,
    rewrite: &Rewrite<RelationName>,
    level: usize,
    parenthesize: bool,
) -> Result<(), PrintError> {
    if parenthesize {
        text.push('(');
        expression(text, rewrite, level)?;
        text.push(')');
    } else {
        expression(text, rewrite, level)?;
    }
    Ok(())
}
