//! A namespace: the declared relations of one kind of resource, their
//! rewrites, and their constraints.
//!
//! A namespace checks itself whole when it is built, so every [`Namespace`] is
//! valid and a stored one always evaluates. [`Namespace::new`] refuses, with
//! every problem it finds:
//!
//! - a relation declared twice;
//! - a rewrite tree deeper than [`MAX_REWRITE_DEPTH`];
//! - a computed subjectset or factset naming a relation the namespace doesn't
//!   declare;
//! - a relation that reads `this` without constraints, or one with
//!   constraints that doesn't read `this`;
//! - more than [`MAX_CONSTRAINTS`] constraints on a relation, or one
//!   constraint declared twice;
//! - a cycle of computed subjectsets, such as `viewer: editor` with
//!   `editor: viewer`, or `viewer: this ! viewer`. Relations that reach
//!   each other, a strongly connected component, are one problem however
//!   many cycles they form.
//!
//! A computed cycle comes back to its relation without reading a fact, so it
//! can never establish a membership. A cycle through a fact-to-subjectset
//! reads a fact at each step, as with folders inside folders, and is left
//! to evaluation.
//!
//! A namespace doesn't know its own name or the other namespaces, so the
//! checks that reach past it, a constraint `namespace#relation` and the
//! relation a fact-to-subjectset follows, are made by
//! [`Namespace::references`], against whatever namespaces the caller knows.

use std::{
    collections::{HashMap, HashSet, VecDeque, hash_map::Entry},
    fmt::{self, Display},
    hash::Hash,
    sync::Arc,
};

use thiserror::Error;

use crate::{
    graph::Subject,
    id::{NamespaceId, RelationId},
    rewrite::Rewrite,
};

/// How deeply rewrite operators may nest. A leaf is one level.
pub const MAX_REWRITE_DEPTH: usize = 100;

/// How many relations a cycle error names before it says how many more.
pub const MAX_CYCLE_NAMES: usize = 8;

/// How many constraints a relation declares at most.
pub const MAX_CONSTRAINTS: usize = 32;

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
    #[error("relation '{0}' reads 'this' and declares no constraints")]
    Unconstrained(String),
    #[error("relation '{0}' declares constraints and doesn't read 'this'")]
    ConstrainedComputed(String),
    #[error("relation '{relation}' declares {count} constraints, more than {MAX_CONSTRAINTS}")]
    TooManyConstraints { relation: String, count: usize },
    #[error("relation '{relation}' declares constraint '{constraint}' more than once")]
    DuplicateConstraint {
        relation: String,
        constraint: String,
    },
    /// A constraint `namespace#relation` whose namespace doesn't declare
    /// the relation.
    #[error(
        "relation '{relation}' has constraint '{constraint}', whose namespace doesn't declare its relation"
    )]
    UndeclaredConstraint {
        relation: String,
        constraint: String,
    },
    /// A fact-to-subjectset whose factset's constraints name a namespace
    /// that doesn't declare the relation it follows.
    #[error(
        "relation '{relation}' follows '{followed}', which namespace '{namespace}' doesn't declare"
    )]
    UndeclaredFollowed {
        relation: String,
        namespace: String,
        followed: String,
    },
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

/// A kind of subject a relation's facts may bind. Generic over how
/// namespaces and relations are named, as [`Namespace`] is.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Constraint<N = NamespaceId, R = RelationId> {
    /// `identity`: an identity.
    Identity,
    /// `namespace`: a resource member of the namespace, `namespace:id`.
    Resource(N),
    /// `namespace#relation`: a subjectset of the namespace's relation.
    Subjectset(N, R),
}

impl<N, R> Constraint<N, R> {
    /// The namespace the constraint names, `None` for an identity.
    pub fn namespace(&self) -> Option<&N> {
        match self {
            Constraint::Identity => None,
            Constraint::Resource(namespace) | Constraint::Subjectset(namespace, _) => {
                Some(namespace)
            }
        }
    }

    /// The constraint with its names passed through `namespace` and
    /// `relation`, stopping at the first error.
    pub fn try_map<M, S, E>(
        &self,
        namespace: &mut impl FnMut(&N) -> Result<M, E>,
        relation: &mut impl FnMut(&R) -> Result<S, E>,
    ) -> Result<Constraint<M, S>, E> {
        Ok(match self {
            Constraint::Identity => Constraint::Identity,
            Constraint::Resource(name) => Constraint::Resource(namespace(name)?),
            Constraint::Subjectset(name, set) => {
                Constraint::Subjectset(namespace(name)?, relation(set)?)
            }
        })
    }
}

impl Constraint {
    /// Whether a fact may bind `subject` under this constraint.
    pub fn admits(&self, subject: &Subject) -> bool {
        match (self, subject) {
            (Constraint::Identity, Subject::Identity(_)) => true,
            (Constraint::Resource(namespace), Subject::ResourceMember(resource)) => {
                resource.namespace == *namespace
            }
            (Constraint::Subjectset(namespace, relation), Subject::Subjectset(set)) => {
                set.resource.namespace == *namespace && set.relation == *relation
            }
            _ => false,
        }
    }
}

/// As a document writes it: `identity`, `namespace` or `namespace#relation`.
impl<N: Display, R: Display> Display for Constraint<N, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Constraint::Identity => f.write_str("identity"),
            Constraint::Resource(namespace) => namespace.fmt(f),
            Constraint::Subjectset(namespace, relation) => write!(f, "{namespace}#{relation}"),
        }
    }
}

/// One relation as it's declared: its name, its rewrite, and its
/// constraints, empty for a computed relation.
pub type Declared<R, N> = (R, Rewrite<R>, Vec<Constraint<N, R>>);

/// A relation as a namespace holds it.
#[derive(Debug)]
struct Relation<R, N> {
    name: R,
    rewrite: Arc<Rewrite<R>>,
    constraints: Vec<Constraint<N, R>>,
}

/// A valid namespace. Generic over how relations and namespaces are named:
/// storage holds interned [`RelationId`]s and [`NamespaceId`]s, and a
/// namespace document names them.
///
/// A namespace may declare no relations. Its resources are then only ever
/// subjects, `namespace:id`, of facts in other namespaces, as a `user` type
/// is in other ReBAC models.
#[derive(Debug)]
pub struct Namespace<R = RelationId, N = NamespaceId> {
    /// In declaration order.
    relations: Vec<Relation<R, N>>,
    index: HashMap<R, usize>,
}

impl<R, N> Namespace<R, N>
where
    R: Eq + Hash + Clone + Display,
    N: Eq + Hash + Clone + Display,
{
    /// The relations, or every problem with them, in declaration order.
    ///
    /// Linear in the namespace's size, its relations, the nodes of their
    /// rewrites and their constraints, with hashing taken as constant: each
    /// relation is indexed once and each rewrite walked once, the cycle
    /// search visits each relation and each computed edge once, and a
    /// cycle's shortest path is searched only within its own component.
    /// Sorting the problems found adds `p log p` for `p` problems.
    pub fn new(relations: Vec<Declared<R, N>>) -> Result<Self, Vec<Problem>> {
        // Each relation at its first declaration; a later one is a problem.
        let mut index = HashMap::new();
        for (at, (relation, ..)) in relations.iter().enumerate() {
            index.entry(relation.clone()).or_insert(at);
        }
        let problems = problems(&relations, &index);
        if !problems.is_empty() {
            return Err(problems);
        }
        let relations = relations
            .into_iter()
            .map(|(name, rewrite, constraints)| Relation {
                name,
                rewrite: Arc::new(rewrite),
                constraints,
            })
            .collect();
        Ok(Self { relations, index })
    }

    /// Refuses a relation named twice.
    fn indexed(relations: Vec<Relation<R, N>>) -> Result<Self, NamespaceError> {
        let mut index = HashMap::new();
        for (at, relation) in relations.iter().enumerate() {
            if index.insert(relation.name.clone(), at).is_some() {
                return Err(NamespaceError::DuplicateRelation(relation.name.to_string()));
            }
        }
        Ok(Self { relations, index })
    }

    /// `None` when the namespace doesn't declare the relation.
    pub fn rewrite(&self, relation: &R) -> Option<&Arc<Rewrite<R>>> {
        self.index
            .get(relation)
            .map(|&at| &self.relations[at].rewrite)
    }

    /// The relation's constraints, in declaration order: empty when it's
    /// computed or the namespace doesn't declare it.
    pub fn constraints(&self, relation: &R) -> &[Constraint<N, R>] {
        self.index
            .get(relation)
            .map_or(&[], |&at| &self.relations[at].constraints)
    }

    /// Every declared relation and its rewrite, in declaration order.
    pub fn relations(&self) -> impl Iterator<Item = (&R, &Rewrite<R>)> {
        self.relations
            .iter()
            .map(|relation| (&relation.name, &*relation.rewrite))
    }

    /// Every relation the namespace names, declared, referenced from a
    /// rewrite or named by a constraint, each once, in the order first met.
    pub fn names(&self) -> Vec<&R> {
        let mut seen = HashSet::new();
        let mut names = Vec::new();
        let mut name = |relation| {
            if seen.insert(relation) {
                names.push(relation);
            }
        };
        for relation in &self.relations {
            name(&relation.name);
            relation.rewrite.visit(&mut name);
            for constraint in &relation.constraints {
                if let Constraint::Subjectset(_, set) = constraint {
                    name(set);
                }
            }
        }
        names
    }

    /// Every namespace the constraints name, each once, in the order first
    /// met.
    pub fn namespaces(&self) -> Vec<&N> {
        let mut seen = HashSet::new();
        self.relations
            .iter()
            .flat_map(|relation| &relation.constraints)
            .filter_map(Constraint::namespace)
            .filter(|namespace| seen.insert(*namespace))
            .collect()
    }

    /// Every namespace whose relations [`references`](Self::references)
    /// asks about, each once, in the order first met: the namespace of each
    /// constraint `namespace#relation`, and each namespace a
    /// fact-to-subjectset's factset takes. A namespace named only by a
    /// plain resource constraint, which nothing judges, isn't one.
    pub fn judged(&self) -> Vec<&N> {
        let mut seen = HashSet::new();
        let mut judged = Vec::new();
        let mut judge = |namespace| {
            if seen.insert(namespace) {
                judged.push(namespace);
            }
        };
        for relation in &self.relations {
            for constraint in &relation.constraints {
                if let Constraint::Subjectset(namespace, _) = constraint {
                    judge(namespace);
                }
            }
            let mut followed = Vec::new();
            fact_tos(&relation.rewrite, &mut followed);
            for (factset, _) in followed {
                let constraints = self.constraints(factset).iter();
                constraints
                    .filter_map(Constraint::namespace)
                    .for_each(&mut judge);
            }
        }
        judged
    }

    /// The same namespace with every relation renamed by `relation` and
    /// every namespace by `namespace`, as when its names are interned.
    /// Renaming doesn't change any rewrite's shape, so a one-to-one rename
    /// keeps the namespace valid. A rename that gives two relations one
    /// name is refused.
    ///
    /// Each rename is called once for each distinct name, and every
    /// reference to a name gets the same result, so a rename that would
    /// answer differently on a second call can't leave a reference
    /// dangling.
    pub fn map<S, M>(
        &self,
        mut relation: impl FnMut(&R) -> S,
        mut namespace: impl FnMut(&N) -> M,
    ) -> Result<Namespace<S, M>, NamespaceError>
    where
        S: Eq + Hash + Clone + Display,
        M: Eq + Hash + Clone + Display,
    {
        self.try_map(
            |name| Ok(relation(name)),
            |name| Ok::<_, NamespaceError>(namespace(name)),
        )
    }

    /// Like [`Namespace::map`], with renames that can fail, as interning
    /// can. The first error stops the rename and is returned.
    pub fn try_map<S, M, E>(
        &self,
        relation: impl FnMut(&R) -> Result<S, E>,
        namespace: impl FnMut(&N) -> Result<M, E>,
    ) -> Result<Namespace<S, M>, E>
    where
        S: Eq + Hash + Clone + Display,
        M: Eq + Hash + Clone + Display,
        E: From<NamespaceError>,
    {
        let mut relation = once(relation);
        let mut namespace = once(namespace);
        let relations = self
            .relations
            .iter()
            .map(|declared| {
                let name = relation(&declared.name)?;
                let rewrite = (*declared.rewrite)
                    .clone()
                    .try_map(&mut |reference| relation(&reference))?;
                let constraints = declared
                    .constraints
                    .iter()
                    .map(|constraint| constraint.try_map(&mut namespace, &mut relation))
                    .collect::<Result<_, E>>()?;
                Ok(Relation {
                    name,
                    rewrite: Arc::new(rewrite),
                    constraints,
                })
            })
            .collect::<Result<_, E>>()?;
        Ok(Namespace::indexed(relations)?)
    }

    /// Every problem with the references that reach past this namespace,
    /// whose name is `own`, in declaration order:
    ///
    /// - a constraint `namespace#relation` whose namespace doesn't declare
    ///   the relation;
    /// - a fact-to-subjectset whose factset's constraints name a namespace
    ///   that doesn't declare the relation it follows, each such namespace
    ///   and relation once for each relation.
    ///
    /// This namespace answers for references to itself. `declares` answers
    /// for every other: whether namespace `N` declares relation `R`, or
    /// `None` when the caller doesn't know that namespace and leaves the
    /// reference to be checked elsewhere.
    pub fn references(&self, own: &N, declares: impl Fn(&N, &R) -> Option<bool>) -> Vec<Problem> {
        let undeclared = |namespace: &N, relation: &R| {
            let declared = if namespace == own {
                Some(self.index.contains_key(relation))
            } else {
                declares(namespace, relation)
            };
            declared == Some(false)
        };
        let mut problems = Vec::new();
        for (at, declared) in self.relations.iter().enumerate() {
            let name = || declared.name.to_string();
            for constraint in &declared.constraints {
                if let Constraint::Subjectset(namespace, relation) = constraint
                    && undeclared(namespace, relation)
                {
                    let error = NamespaceError::UndeclaredConstraint {
                        relation: name(),
                        constraint: constraint.to_string(),
                    };
                    problems.push(Problem { at, error });
                }
            }
            let mut followed = Vec::new();
            fact_tos(&declared.rewrite, &mut followed);
            let mut reported = HashSet::new();
            for (factset, computed) in followed {
                let namespaces = self.constraints(factset).iter();
                for namespace in namespaces.filter_map(Constraint::namespace) {
                    if undeclared(namespace, computed) && reported.insert((namespace, computed)) {
                        let error = NamespaceError::UndeclaredFollowed {
                            relation: name(),
                            namespace: namespace.to_string(),
                            followed: computed.to_string(),
                        };
                        problems.push(Problem { at, error });
                    }
                }
            }
        }
        problems
    }
}

/// `rename`, asked once for each distinct name, every later call given
/// the first answer.
fn once<A: Eq + Hash + Clone, B: Clone, E>(
    mut rename: impl FnMut(&A) -> Result<B, E>,
) -> impl FnMut(&A) -> Result<B, E> {
    let mut renamed = HashMap::new();
    move |name: &A| {
        if let Some(answer) = renamed.get(name) {
            return Ok(B::clone(answer));
        }
        let answer = rename(name)?;
        renamed.insert(name.clone(), answer.clone());
        Ok(answer)
    }
}

/// Every problem in a whole namespace, as the [module](self) describes, in
/// declaration order. A relation's dangling targets are each reported once,
/// and a cycle at the relation of its component the check meets first.
/// `declared` holds each relation at its first declaration.
fn problems<R: Eq + Hash + Display, N: Eq + Hash + Display>(
    relations: &[Declared<R, N>],
    declared: &HashMap<R, usize>,
) -> Vec<Problem> {
    let mut problems = Vec::new();
    let mut problem = |at, error| problems.push(Problem { at, error });
    for (at, (relation, ..)) in relations.iter().enumerate() {
        if declared.get(relation) != Some(&at) {
            problem(at, NamespaceError::DuplicateRelation(relation.to_string()));
        }
    }
    let mut edges = vec![Vec::new(); relations.len()];
    for (at, (relation, rewrite, constraints)) in relations.iter().enumerate() {
        for error in constraint_problems(relation, constraints) {
            problem(at, error);
        }
        if let Err(error) = check_tree(rewrite, 1) {
            // A tree past the depth limit isn't walked further, so one built
            // in code, however deep, can't overflow the stack.
            problem(at, error);
            continue;
        }
        let mut computed = Vec::new();
        let mut factsets = Vec::new();
        let reads_this = named(rewrite, &mut computed, &mut factsets);
        if reads_this && constraints.is_empty() {
            problem(at, NamespaceError::Unconstrained(relation.to_string()));
        } else if !reads_this && !constraints.is_empty() {
            problem(
                at,
                NamespaceError::ConstrainedComputed(relation.to_string()),
            );
        }
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

/// Refuses more than [`MAX_CONSTRAINTS`] constraints, and reports each
/// constraint declared twice once. Past the limit, nothing more is checked.
fn constraint_problems<R: Eq + Hash + Display, N: Eq + Hash + Display>(
    relation: &R,
    constraints: &[Constraint<N, R>],
) -> Vec<NamespaceError> {
    if constraints.len() > MAX_CONSTRAINTS {
        return vec![NamespaceError::TooManyConstraints {
            relation: relation.to_string(),
            count: constraints.len(),
        }];
    }
    let mut seen = HashSet::new();
    let mut twice = HashSet::new();
    constraints
        .iter()
        .filter(|constraint| !seen.insert(*constraint) && twice.insert(*constraint))
        .map(|constraint| NamespaceError::DuplicateConstraint {
            relation: relation.to_string(),
            constraint: constraint.to_string(),
        })
        .collect()
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
/// subjectsets into `computed`, and factsets into `factsets`. Whether the
/// rewrite reads `this`.
fn named<'r, R>(
    rewrite: &'r Rewrite<R>,
    computed: &mut Vec<&'r R>,
    factsets: &mut Vec<&'r R>,
) -> bool {
    match rewrite {
        Rewrite::This => true,
        Rewrite::Computed(relation) => {
            computed.push(relation);
            false
        }
        Rewrite::FactTo { factset, .. } => {
            factsets.push(factset);
            false
        }
        Rewrite::Union(operands) | Rewrite::Intersection(operands) => {
            let mut this = false;
            for operand in operands {
                this |= named(operand, computed, factsets);
            }
            this
        }
        Rewrite::Exclusion(base, excluded) => {
            let base = named(base, computed, factsets);
            named(excluded, computed, factsets) || base
        }
    }
}

/// Each `(factset, computed)` pair of the fact-to-subjectsets in `rewrite`.
fn fact_tos<'r, R>(rewrite: &'r Rewrite<R>, found: &mut Vec<(&'r R, &'r R)>) {
    match rewrite {
        Rewrite::This | Rewrite::Computed(_) => {}
        Rewrite::FactTo { factset, computed } => found.push((factset, computed)),
        Rewrite::Union(operands) | Rewrite::Intersection(operands) => {
            for operand in operands {
                fact_tos(operand, found);
            }
        }
        Rewrite::Exclusion(base, excluded) => {
            fact_tos(base, found);
            fact_tos(excluded, found);
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

    /// Relation `relation`, reading `this` and taking identities.
    fn stored(relation: i32) -> Declared<i32, &'static str> {
        (relation, This, vec![Constraint::Identity])
    }

    #[test]
    fn renaming_keeps_order_rewrites_and_constraints() {
        let namespace = Namespace::new(vec![
            (2, This, vec![Constraint::Subjectset("group", 1)]),
            (0, Computed(2), vec![]),
            stored(1),
        ])
        .unwrap();
        let renamed = namespace
            .map(|relation| relation + 10, |namespace| namespace.len())
            .unwrap();
        let order: Vec<_> = renamed.relations().map(|(relation, _)| *relation).collect();
        assert_eq!(order, [12, 10, 11]);
        assert_eq!(
            renamed.rewrite(&10).map(|rewrite| &**rewrite),
            Some(&Computed(12))
        );
        assert!(renamed.rewrite(&0).is_none());
        assert_eq!(renamed.constraints(&12), [Constraint::Subjectset(5, 11)]);
        assert_eq!(renamed.constraints(&10), []);
        assert_eq!(renamed.constraints(&0), []);
    }

    #[test]
    fn a_rename_is_asked_once_per_name() {
        let namespace = Namespace::new(vec![
            (
                0,
                This,
                vec![Constraint::Resource("a"), Constraint::Subjectset("a", 0)],
            ),
            (1, Computed(0), vec![]),
        ])
        .unwrap();
        let (mut relations, mut namespaces) = (0, 0);
        let renamed = namespace
            .map(
                |_| {
                    relations += 1;
                    relations
                },
                |_| {
                    namespaces += 1;
                    namespaces
                },
            )
            .unwrap();
        assert_eq!((relations, namespaces), (2, 1));
        assert_eq!(
            renamed.rewrite(&2).map(|rewrite| &**rewrite),
            Some(&Computed(1))
        );
        assert_eq!(
            renamed.constraints(&1),
            [Constraint::Resource(1), Constraint::Subjectset(1, 1)]
        );
    }

    #[test]
    fn renaming_two_relations_alike_is_refused() {
        let namespace = Namespace::new(vec![stored(0), stored(1)]).unwrap();
        assert_eq!(
            namespace.map(|_| 0, |namespace| *namespace).unwrap_err(),
            NamespaceError::DuplicateRelation("0".to_owned())
        );
    }

    #[test]
    fn a_failed_rename_stops_at_its_error() {
        let namespace = Namespace::new(vec![(0, This, vec![Constraint::Resource("a")])]).unwrap();
        let renamed = namespace.try_map(
            |relation| Ok(*relation),
            |_| Err::<i32, _>(NamespaceError::TooDeep),
        );
        assert_eq!(renamed.unwrap_err(), NamespaceError::TooDeep);
    }
}
