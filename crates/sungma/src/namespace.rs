//! A namespace: the declared relations of one kind of resource, and their
//! rewrites.
//!
//! A namespace checks itself whole when it is built, so every [`Namespace`] is
//! valid and a stored one always evaluates. [`Namespace::new`] refuses, with
//! every problem it finds:
//!
//! - a relation declared twice;
//! - a rewrite tree deeper than [`MAX_REWRITE_DEPTH`];
//! - a computed subjectset or factset naming a relation the namespace doesn't
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
//! namespace of the resource the fact names, so it isn't checked here.

use std::{
    collections::{HashMap, HashSet, VecDeque, hash_map::Entry},
    fmt::Display,
    hash::Hash,
    sync::Arc,
};

use thiserror::Error;

use crate::{id::RelationId, rewrite::Rewrite};

/// How deeply rewrite operators may nest. A leaf is one level.
pub const MAX_REWRITE_DEPTH: usize = 100;

/// How many relations a cycle error names before it says how many more.
pub const MAX_CYCLE_NAMES: usize = 8;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum NamespaceError {
    #[error("a rewrite tree deeper than {MAX_REWRITE_DEPTH} levels is refused")]
    TooDeep,
    #[error("relation '{0}' is declared more than once")]
    DuplicateRelation(String),
    #[error("relation '{relation}' references '{target}', which the namespace doesn't declare")]
    DanglingReference { relation: String, target: String },
    /// A path from a relation back to itself, naming at most
    /// [`MAX_CYCLE_NAMES`] relations on the way and counting the rest.
    #[error("rewrite cycle: {}", cycle(.path, *.omitted))]
    RewriteCycle { path: Vec<String>, omitted: usize },
}

fn cycle(path: &[String], omitted: usize) -> String {
    match path.split_last() {
        Some((root, way)) if omitted > 0 => {
            format!("{} -> ... {omitted} more -> {root}", way.join(" -> "))
        }
        _ => path.join(" -> "),
    }
}

/// A problem with a namespace, at the position of the relation it was found
/// at, counting from 0 in declaration order.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("{error}")]
pub struct Problem {
    pub at: usize,
    pub error: NamespaceError,
}

/// A valid namespace. Generic over how relations are named: storage holds
/// interned [`RelationId`]s, and a namespace document names its relations.
///
/// A namespace may declare no relations. Its resources are then only ever
/// subjects, `namespace:id`, of facts in other namespaces, as a `user` type
/// is in other ReBAC models.
#[derive(Debug)]
pub struct Namespace<R = RelationId> {
    /// In declaration order.
    relations: Vec<(R, Arc<Rewrite<R>>)>,
    index: HashMap<R, usize>,
}

impl<R: Eq + Hash + Clone + Display> Namespace<R> {
    /// The relations, or every problem with them, in declaration order.
    ///
    /// Linear in the namespace's size, its relations and the nodes of their
    /// rewrites, with hashing taken as constant: each relation is indexed
    /// once and each rewrite walked once, the cycle search visits each
    /// relation and each computed edge once, and a cycle's shortest path is
    /// searched only within its own component. Sorting the problems found
    /// adds `p log p` for `p` problems.
    pub fn new(relations: Vec<(R, Rewrite<R>)>) -> Result<Self, Vec<Problem>> {
        // Each relation at its first declaration; a later one is a problem.
        let mut index = HashMap::new();
        for (at, (relation, _)) in relations.iter().enumerate() {
            index.entry(relation.clone()).or_insert(at);
        }
        let problems = problems(&relations, &index);
        if !problems.is_empty() {
            return Err(problems);
        }
        let relations = relations
            .into_iter()
            .map(|(relation, rewrite)| (relation, Arc::new(rewrite)))
            .collect();
        Ok(Self { relations, index })
    }

    /// Refuses a relation named twice.
    fn indexed(relations: Vec<(R, Arc<Rewrite<R>>)>) -> Result<Self, NamespaceError> {
        let mut index = HashMap::new();
        for (at, (relation, _)) in relations.iter().enumerate() {
            if index.insert(relation.clone(), at).is_some() {
                return Err(NamespaceError::DuplicateRelation(relation.to_string()));
            }
        }
        Ok(Self { relations, index })
    }

    /// `None` when the namespace doesn't declare the relation.
    pub fn rewrite(&self, relation: &R) -> Option<&Arc<Rewrite<R>>> {
        self.index.get(relation).map(|&at| &self.relations[at].1)
    }

    /// Every declared relation and its rewrite, in declaration order.
    pub fn relations(&self) -> impl Iterator<Item = (&R, &Rewrite<R>)> {
        self.relations
            .iter()
            .map(|(relation, rewrite)| (relation, &**rewrite))
    }

    /// Every relation the namespace names, declared or referenced from a
    /// rewrite, each once, in the order first met.
    pub fn names(&self) -> Vec<&R> {
        let mut seen = HashSet::new();
        let mut names = Vec::new();
        let mut name = |relation| {
            if seen.insert(relation) {
                names.push(relation);
            }
        };
        for (relation, rewrite) in &self.relations {
            name(relation);
            rewrite.visit(&mut name);
        }
        names
    }

    /// The same namespace with every relation renamed by `rename`, as when its
    /// names are interned. Renaming doesn't change any rewrite's shape, so a
    /// one-to-one rename keeps the namespace valid. A rename that gives two
    /// relations one name is refused.
    ///
    /// `rename` is called once for each distinct name, and every reference
    /// to a name gets the same result, so a rename that would answer
    /// differently on a second call can't leave a reference dangling.
    pub fn map<S: Eq + Hash + Clone + Display>(
        &self,
        mut rename: impl FnMut(&R) -> S,
    ) -> Result<Namespace<S>, NamespaceError> {
        self.try_map(|relation| Ok(rename(relation)))
    }

    /// Like [`Namespace::map`], with a rename that can fail, as interning
    /// can. The first error stops the rename and is returned.
    pub fn try_map<S, E>(
        &self,
        mut rename: impl FnMut(&R) -> Result<S, E>,
    ) -> Result<Namespace<S>, E>
    where
        S: Eq + Hash + Clone + Display,
        E: From<NamespaceError>,
    {
        let mut renamed = HashMap::new();
        let mut name = |relation: &R| -> Result<S, E> {
            if let Some(name) = renamed.get(relation) {
                return Ok(S::clone(name));
            }
            let name = rename(relation)?;
            renamed.insert(relation.clone(), name.clone());
            Ok(name)
        };
        let relations = self
            .relations
            .iter()
            .map(|(relation, rewrite)| {
                let declared = name(relation)?;
                let rewrite = (**rewrite)
                    .clone()
                    .try_map(&mut |reference| name(&reference))?;
                Ok((declared, Arc::new(rewrite)))
            })
            .collect::<Result<_, E>>()?;
        Ok(Namespace::indexed(relations)?)
    }
}

/// Every problem in a whole namespace, as the [module](self) describes, in
/// declaration order. A relation's dangling targets are each reported once,
/// and a cycle at the relation of its component the check meets first.
/// `declared` holds each relation at its first declaration.
fn problems<R: Eq + Hash + Display>(
    relations: &[(R, Rewrite<R>)],
    declared: &HashMap<R, usize>,
) -> Vec<Problem> {
    let mut problems = Vec::new();
    let mut problem = |at, error| problems.push(Problem { at, error });
    for (at, (relation, _)) in relations.iter().enumerate() {
        if declared.get(relation) != Some(&at) {
            problem(at, NamespaceError::DuplicateRelation(relation.to_string()));
        }
    }
    let mut edges = vec![Vec::new(); relations.len()];
    for (at, (relation, rewrite)) in relations.iter().enumerate() {
        if let Err(error) = check_tree(rewrite, 1) {
            // A tree past the depth limit isn't walked further, so one built
            // in code, however deep, can't overflow the stack.
            problem(at, error);
            continue;
        }
        let mut computed = Vec::new();
        let mut factsets = Vec::new();
        named(rewrite, &mut computed, &mut factsets);
        let mut dangling = HashSet::new();
        for target in computed.iter().chain(&factsets) {
            if !declared.contains_key(target) && dangling.insert(*target) {
                problem(
                    at,
                    NamespaceError::DanglingReference {
                        relation: relation.to_string(),
                        target: target.to_string(),
                    },
                );
            }
        }
        if declared.get(relation) == Some(&at) {
            edges[at] = computed
                .iter()
                .filter_map(|target| declared.get(target).copied())
                .collect();
        }
    }
    let mut components = Components::new(&edges);
    for at in 0..relations.len() {
        components.visit(at);
    }
    for cycle in components.cycles {
        let named = cycle.len() - 1;
        let shown = named.min(MAX_CYCLE_NAMES);
        let mut path: Vec<_> = cycle[..shown]
            .iter()
            .map(|&at| relations[at].0.to_string())
            .collect();
        path.push(relations[cycle[0]].0.to_string());
        problem(
            cycle[0],
            NamespaceError::RewriteCycle {
                path,
                omitted: named - shown,
            },
        );
    }
    problems.sort_by_key(|problem| problem.at);
    problems
}

/// Refuses nesting past [`MAX_REWRITE_DEPTH`].
/// `level` is the depth of `rewrite`, the root being 1.
fn check_tree<R>(rewrite: &Rewrite<R>, level: usize) -> Result<(), NamespaceError> {
    if level > MAX_REWRITE_DEPTH {
        return Err(NamespaceError::TooDeep);
    }
    match rewrite {
        Rewrite::This | Rewrite::Computed(_) | Rewrite::FactTo { .. } => Ok(()),
        Rewrite::Union(operands) | Rewrite::Intersection(operands) => operands
            .iter()
            .try_for_each(|operand| check_tree(operand, level + 1)),
        Rewrite::Exclusion(base, excluded) => {
            check_tree(base, level + 1)?;
            check_tree(excluded, level + 1)
        }
    }
}

/// The relations of its own namespace that `rewrite` names: computed
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
        let mut members = HashSet::new();
        while let Some(member) = self.open.pop() {
            self.on_open[member] = false;
            members.insert(member);
            if member == root {
                break;
            }
        }
        if let Some(cycle) = cycle_through(self.edges, root, &members) {
            self.cycles.push(cycle);
        }
    }
}

/// The shortest path from `root` back to itself, searching only the
/// `members` of its component, so the search costs no more than the
/// component. `None` when there's none: a lone relation that doesn't name
/// itself.
fn cycle_through(
    edges: &[Vec<usize>],
    root: usize,
    members: &HashSet<usize>,
) -> Option<Vec<usize>> {
    let mut from = HashMap::new();
    let mut queue = VecDeque::from([root]);
    while let Some(at) = queue.pop_front() {
        for &target in &edges[at] {
            if target == root {
                let mut path = vec![at];
                let mut step = at;
                while let Some(&previous) = from.get(&step) {
                    path.push(previous);
                    step = previous;
                }
                path.reverse();
                path.push(root);
                return Some(path);
            }
            if !members.contains(&target) {
                continue;
            }
            if let Entry::Vacant(entry) = from.entry(target) {
                entry.insert(at);
                queue.push_back(target);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rewrite::Rewrite::{Computed, This};

    #[test]
    fn renaming_keeps_order_and_rewrites() {
        let namespace = Namespace::new(vec![(2, This), (0, Computed(2)), (1, This)]).unwrap();
        let renamed = namespace.map(|relation| relation + 10).unwrap();
        let order: Vec<_> = renamed.relations().map(|(relation, _)| *relation).collect();
        assert_eq!(order, [12, 10, 11]);
        assert_eq!(
            renamed.rewrite(&10).map(|rewrite| &**rewrite),
            Some(&Computed(12))
        );
        assert!(renamed.rewrite(&0).is_none());
    }

    #[test]
    fn a_rename_is_asked_once_per_name() {
        let namespace = Namespace::new(vec![(0, This), (1, Computed(0))]).unwrap();
        let mut calls = 0;
        let renamed = namespace
            .map(|_| {
                calls += 1;
                calls
            })
            .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(
            renamed.rewrite(&2).map(|rewrite| &**rewrite),
            Some(&Computed(1))
        );
    }

    #[test]
    fn renaming_two_relations_alike_is_refused() {
        let namespace = Namespace::new(vec![(0, This), (1, This)]).unwrap();
        assert_eq!(
            namespace.map(|_| 0).unwrap_err(),
            NamespaceError::DuplicateRelation("0".to_owned())
        );
    }
}
