//! The rewrite tree a relation evaluates.

use serde::Deserialize;

use crate::model::RelationId;

/// A relation's rewrite.
///
/// Generic over how relations are named: the core uses interned
/// [`RelationId`]s, and tests write `Rewrite<&str>` and intern it with
/// [`Rewrite::map`].
///
/// In JSON each node is externally tagged: `"this"`, `{"computed": "owner"}`,
/// `{"fact_to": {"factset": "parent", "computed": "viewer"}}`,
/// `{"union": [..]}`, `{"intersection": [..]}` and
/// `{"exclusion": [base, excluded]}`.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rewrite<R = RelationId> {
    /// The subjects of the facts stored under the subjectset in hand.
    This,
    /// A relation evaluated on the resource in hand.
    Computed(R),
    /// `(factset, computed)`: read the facts under `factset`, then evaluate
    /// `computed` on each resource they name.
    FactTo {
        factset: R,
        computed: R,
    },
    Union(Vec<Rewrite<R>>),
    Intersection(Vec<Rewrite<R>>),
    Exclusion(Box<Rewrite<R>>, Box<Rewrite<R>>),
}

impl<R> Rewrite<R> {
    /// Rebuilds the tree with every relation name passed through `f`.
    pub fn map<S>(self, f: &mut impl FnMut(R) -> S) -> Rewrite<S> {
        match self {
            Rewrite::This => Rewrite::This,
            Rewrite::Computed(relation) => Rewrite::Computed(f(relation)),
            Rewrite::FactTo { factset, computed } => Rewrite::FactTo {
                factset: f(factset),
                computed: f(computed),
            },
            Rewrite::Union(operands) => {
                Rewrite::Union(operands.into_iter().map(|r| r.map(f)).collect())
            }
            Rewrite::Intersection(operands) => {
                Rewrite::Intersection(operands.into_iter().map(|r| r.map(f)).collect())
            }
            Rewrite::Exclusion(base, excluded) => {
                Rewrite::Exclusion(Box::new(base.map(f)), Box::new(excluded.map(f)))
            }
        }
    }
}
