//! A theory: the declared relations of one kind of resource, and their
//! rewrites.
//!
//! A theory is checked whole when it is declared, so a stored theory always
//! evaluates. [`validate`] refuses:
//!
//! - a relation declared twice;
//! - an empty union or intersection;
//! - a rewrite tree deeper than [`MAX_REWRITE_DEPTH`];
//! - a computed subjectset or factset naming a relation the theory doesn't
//!   declare;
//! - a cycle of computed subjectsets, such as `viewer: editor` with
//!   `editor: viewer`, or `viewer: this ! viewer`.
//!
//! A computed cycle comes back to its relation without reading a fact, so it
//! can never establish a membership. A cycle through a fact-to-subjectset
//! reads a fact at each step, as with folders inside folders, and is left
//! to evaluation. The relation a fact-to-subjectset computes belongs to the
//! theory of the resource the fact names, so it isn't checked here.

use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
    hash::Hash,
    sync::Arc,
};

use thiserror::Error;

use crate::{model::RelationId, rewrite::Rewrite};

/// How deeply rewrite operators may nest. A leaf is one level.
pub const MAX_REWRITE_DEPTH: usize = 100;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TheoryError {
    #[error("an empty {0} has no operands")]
    EmptyOperator(&'static str),
    #[error("a rewrite tree deeper than {MAX_REWRITE_DEPTH} levels is refused")]
    TooDeep,
    #[error("relation '{0}' is declared more than once")]
    DuplicateRelation(String),
    #[error("relation '{relation}' references '{target}', which the theory doesn't declare")]
    DanglingReference { relation: String, target: String },
    #[error("rewrite cycle: {}", .0.join(" -> "))]
    RewriteCycle(Vec<String>),
}

/// A relation declared without a rewrite is declared with [`Rewrite::This`].
#[derive(Debug)]
pub struct Theory {
    relations: HashMap<RelationId, Arc<Rewrite>>,
}

impl Theory {
    /// The relations, if [`validate`] accepts them.
    pub fn new(relations: Vec<(RelationId, Rewrite)>) -> Result<Self, TheoryError> {
        validate(&relations)?;
        let relations = relations
            .into_iter()
            .map(|(relation, rewrite)| (relation, Arc::new(rewrite)))
            .collect();
        Ok(Self { relations })
    }

    /// `None` when the theory doesn't declare the relation.
    pub fn rewrite(&self, relation: RelationId) -> Option<&Arc<Rewrite>> {
        self.relations.get(&relation)
    }

    /// Every declared relation and its rewrite, in no order.
    pub fn relations(&self) -> impl Iterator<Item = (RelationId, &Rewrite)> {
        self.relations
            .iter()
            .map(|(relation, rewrite)| (*relation, &**rewrite))
    }
}

/// Checks a whole theory, as the [module](self) describes, and refuses it
/// with the first problem in declaration order. Generic over how relations
/// are named, so a theory can be checked by name before it is interned.
pub fn validate<R: Eq + Hash + Display>(relations: &[(R, Rewrite<R>)]) -> Result<(), TheoryError> {
    let mut declared = HashSet::new();
    for (relation, _) in relations {
        if !declared.insert(relation) {
            return Err(TheoryError::DuplicateRelation(relation.to_string()));
        }
    }
    let mut edges = HashMap::new();
    for (relation, rewrite) in relations {
        check_tree(rewrite, 1)?;
        let mut computed = Vec::new();
        let mut factsets = Vec::new();
        named(rewrite, &mut computed, &mut factsets);
        if let Some(target) = computed
            .iter()
            .chain(&factsets)
            .find(|target| !declared.contains(*target))
        {
            return Err(TheoryError::DanglingReference {
                relation: relation.to_string(),
                target: target.to_string(),
            });
        }
        edges.insert(relation, computed);
    }
    let mut finished = HashSet::new();
    for (relation, _) in relations {
        let mut path = Vec::new();
        if let Some(cycle) = find_cycle(relation, &edges, &mut path, &mut finished) {
            return Err(TheoryError::RewriteCycle(
                cycle.iter().map(ToString::to_string).collect(),
            ));
        }
    }
    Ok(())
}

/// Refuses empty operators and nesting past [`MAX_REWRITE_DEPTH`].
/// `level` is the depth of `rewrite`, the root being 1.
fn check_tree<R>(rewrite: &Rewrite<R>, level: usize) -> Result<(), TheoryError> {
    if level > MAX_REWRITE_DEPTH {
        return Err(TheoryError::TooDeep);
    }
    match rewrite {
        Rewrite::This | Rewrite::Computed(_) | Rewrite::FactTo { .. } => Ok(()),
        Rewrite::Union(operands) if operands.is_empty() => Err(TheoryError::EmptyOperator("union")),
        Rewrite::Intersection(operands) if operands.is_empty() => {
            Err(TheoryError::EmptyOperator("intersection"))
        }
        Rewrite::Union(operands) | Rewrite::Intersection(operands) => operands
            .iter()
            .try_for_each(|operand| check_tree(operand, level + 1)),
        Rewrite::Exclusion(base, excluded) => {
            check_tree(base, level + 1)?;
            check_tree(excluded, level + 1)
        }
    }
}

/// The relations of its own theory that `rewrite` names: computed
/// subjectsets into `computed`, and factsets into `factsets`.
fn named<'r, R>(rewrite: &'r Rewrite<R>, computed: &mut Vec<&'r R>, factsets: &mut Vec<&'r R>) {
    match rewrite {
        Rewrite::This => {}
        Rewrite::Computed(relation) => computed.push(relation),
        Rewrite::FactTo { factset, .. } => factsets.push(factset),
        Rewrite::Union(operands) | Rewrite::Intersection(operands) => {
            for operand in operands {
                named(operand, computed, factsets);
            }
        }
        Rewrite::Exclusion(base, excluded) => {
            named(base, computed, factsets);
            named(excluded, computed, factsets);
        }
    }
}

/// Depth-first along computed edges from `relation`. `path` holds the
/// relations being visited; meeting one again closes a cycle, returned from
/// its first visit back to itself. `finished` relations reach no cycle.
fn find_cycle<'r, R: Eq + Hash>(
    relation: &'r R,
    edges: &HashMap<&'r R, Vec<&'r R>>,
    path: &mut Vec<&'r R>,
    finished: &mut HashSet<&'r R>,
) -> Option<Vec<&'r R>> {
    if finished.contains(relation) {
        return None;
    }
    if let Some(start) = path.iter().position(|step| *step == relation) {
        let mut cycle = path[start..].to_vec();
        cycle.push(relation);
        return Some(cycle);
    }
    path.push(relation);
    for &target in &edges[relation] {
        if let Some(cycle) = find_cycle(target, edges, path, finished) {
            return Some(cycle);
        }
    }
    path.pop();
    finished.insert(relation);
    None
}
