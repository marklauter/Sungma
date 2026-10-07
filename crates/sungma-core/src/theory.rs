//! A theory: the declared relations of one kind of resource, and their
//! rewrites.
//!
//! A theory is checked whole when it is declared, so a stored theory always
//! evaluates. [`problems`] finds, and [`validate`] refuses:
//!
//! - a relation declared twice;
//! - an empty union or intersection;
//! - a rewrite tree deeper than [`MAX_REWRITE_DEPTH`];
//! - a computed subjectset or factset naming a relation the theory doesn't
//!   declare;
//! - a cycle of computed subjectsets, such as `viewer: editor` with
//!   `editor: viewer`, or `viewer: this ! viewer`. Relations that reach
//!   each other, a strongly connected component, are one problem however
//!   many cycles they form.
//!
//! A computed cycle comes back to its relation without reading a fact, so it
//! can never establish a membership. A cycle through a fact-to-subjectset
//! reads a fact at each step, as with folders inside folders, and is left
//! to evaluation. The relation a fact-to-subjectset computes belongs to the
//! theory of the resource the fact names, so it isn't checked here.

use std::{
    collections::{HashMap, HashSet, VecDeque, hash_map::Entry},
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
/// with its first problem. Generic over how relations are named, so a
/// theory can be checked by name before it is interned.
pub fn validate<R: Eq + Hash + Display>(relations: &[(R, Rewrite<R>)]) -> Result<(), TheoryError> {
    match problems(relations).into_iter().next() {
        Some((_, error)) => Err(error),
        None => Ok(()),
    }
}

/// Every problem in a whole theory, each with the index of the relation it
/// was found at, in declaration order. A relation's dangling targets are
/// each reported once, and a cycle at the relation of its component the
/// check meets first, with a path from that relation back to itself.
pub fn problems<R: Eq + Hash + Display>(
    relations: &[(R, Rewrite<R>)],
) -> Vec<(usize, TheoryError)> {
    let mut problems = Vec::new();
    let mut declared = HashMap::new();
    for (at, (relation, _)) in relations.iter().enumerate() {
        if declared.contains_key(relation) {
            problems.push((at, TheoryError::DuplicateRelation(relation.to_string())));
        } else {
            declared.insert(relation, at);
        }
    }
    let mut edges = vec![Vec::new(); relations.len()];
    for (at, (relation, rewrite)) in relations.iter().enumerate() {
        if let Err(error) = check_tree(rewrite, 1) {
            // A tree past the depth limit isn't walked further, so one built
            // in code, however deep, can't overflow the stack.
            problems.push((at, error));
            continue;
        }
        let mut computed = Vec::new();
        let mut factsets = Vec::new();
        named(rewrite, &mut computed, &mut factsets);
        let mut dangling = HashSet::new();
        for target in computed.iter().chain(&factsets) {
            if !declared.contains_key(*target) && dangling.insert(*target) {
                problems.push((
                    at,
                    TheoryError::DanglingReference {
                        relation: relation.to_string(),
                        target: target.to_string(),
                    },
                ));
            }
        }
        if declared[relation] == at {
            edges[at] = computed
                .iter()
                .filter_map(|target| declared.get(*target).copied())
                .collect();
        }
    }
    let mut components = Components::new(&edges);
    for at in 0..relations.len() {
        components.visit(at);
    }
    for cycle in components.cycles {
        let path = cycle
            .iter()
            .map(|&at| relations[at].0.to_string())
            .collect();
        problems.push((cycle[0], TheoryError::RewriteCycle(path)));
    }
    problems.sort_by_key(|(at, _)| *at);
    problems
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

/// Tarjan's strongly connected components over computed edges, collecting
/// a cycle through each component that has one.
struct Components<'e> {
    edges: &'e [Vec<usize>],
    /// The order each relation was met in, `None` before it is.
    met: Vec<Option<usize>>,
    /// The earliest relation met that each reaches and is still open.
    low: Vec<usize>,
    open: Vec<usize>,
    on_open: Vec<bool>,
    count: usize,
    cycles: Vec<Vec<usize>>,
}

impl<'e> Components<'e> {
    fn new(edges: &'e [Vec<usize>]) -> Self {
        Self {
            edges,
            met: vec![None; edges.len()],
            low: vec![0; edges.len()],
            open: Vec::new(),
            on_open: vec![false; edges.len()],
            count: 0,
            cycles: Vec::new(),
        }
    }

    /// Depth-first from `start`, with its own stack rather than recursion,
    /// so a long chain of relations built in code can't overflow the stack.
    /// Each step consumes an edge, so the walk always ends.
    fn visit(&mut self, start: usize) {
        if self.met[start].is_some() {
            return;
        }
        let edges = self.edges;
        // Each relation being visited, and the edges it has left to follow.
        let mut path = Vec::new();
        let mut entering = Some(start);
        loop {
            if let Some(at) = entering.take() {
                self.met[at] = Some(self.count);
                self.low[at] = self.count;
                self.count += 1;
                self.open.push(at);
                self.on_open[at] = true;
                path.push((at, edges[at].iter()));
            }
            let Some((at, rest)) = path.last_mut() else {
                return;
            };
            let at = *at;
            if let Some(&target) = rest.next() {
                match self.met[target] {
                    None => entering = Some(target),
                    Some(met) if self.on_open[target] => self.low[at] = self.low[at].min(met),
                    Some(_) => {}
                }
                continue;
            }
            path.pop();
            if let Some((parent, _)) = path.last() {
                self.low[*parent] = self.low[*parent].min(self.low[at]);
            }
            if Some(self.low[at]) == self.met[at] {
                self.close_component(at);
            }
        }
    }

    /// Pops the component `root` met first, keeping a cycle through it.
    fn close_component(&mut self, root: usize) {
        let mut members = 0;
        loop {
            let member = self.open.pop().expect("a component's root is open");
            self.on_open[member] = false;
            members += 1;
            if member == root {
                break;
            }
        }
        if members > 1 || self.edges[root].contains(&root) {
            self.cycles.push(cycle_through(self.edges, root));
        }
    }
}

/// The shortest path from `root` back to itself. Only relations in its
/// component lead back to it, so the path stays within the component.
fn cycle_through(edges: &[Vec<usize>], root: usize) -> Vec<usize> {
    let mut from = HashMap::new();
    let mut queue = VecDeque::from([root]);
    while let Some(at) = queue.pop_front() {
        for &target in &edges[at] {
            if target == root {
                let mut path = vec![at];
                while let Some(&previous) = from.get(path.last().unwrap()) {
                    path.push(previous);
                }
                path.reverse();
                path.push(root);
                return path;
            }
            if let Entry::Vacant(entry) = from.entry(target) {
                entry.insert(at);
                queue.push_back(target);
            }
        }
    }
    unreachable!("a component with a cycle has a path back to its root")
}
