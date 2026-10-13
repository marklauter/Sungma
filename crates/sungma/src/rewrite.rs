//! The rewrite tree a relation evaluates.

use std::convert::Infallible;

use crate::{id::RelationId, non_empty::NonEmpty};

/// A relation's rewrite.
///
/// Generic over how relations are named: the core uses interned
/// [`RelationId`]s, and tests write `Rewrite<&str>` and intern it with
/// [`Rewrite::map`]. A namespace document writes one as an expression, as
/// `sungma_lang::namespace` describes.
#[derive(Clone, PartialEq, Eq, Debug)]
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
    /// Has at least one operand, so an empty union can't be built.
    Union(NonEmpty<Rewrite<R>>),
    /// Has at least one operand, so an empty intersection can't be built.
    Intersection(NonEmpty<Rewrite<R>>),
    Exclusion(Box<Rewrite<R>>, Box<Rewrite<R>>),
}

impl<R> Rewrite<R> {
    /// Rebuilds the tree with every relation name passed through `f`.
    pub fn map<S>(self, f: &mut impl FnMut(R) -> S) -> Rewrite<S> {
        let Ok(rewrite) = self.try_map(&mut |relation| Ok::<_, Infallible>(f(relation)));
        rewrite
    }

    /// Like [`Rewrite::map`], stopping at the first error.
    pub fn try_map<S, E>(self, f: &mut impl FnMut(R) -> Result<S, E>) -> Result<Rewrite<S>, E> {
        Ok(match self {
            Rewrite::This => Rewrite::This,
            Rewrite::Computed(relation) => Rewrite::Computed(f(relation)?),
            Rewrite::FactTo { factset, computed } => Rewrite::FactTo {
                factset: f(factset)?,
                computed: f(computed)?,
            },
            Rewrite::Union(operands) => Rewrite::Union(operands.try_map(|r| r.try_map(f))?),
            Rewrite::Intersection(operands) => {
                Rewrite::Intersection(operands.try_map(|r| r.try_map(f))?)
            }
            Rewrite::Exclusion(base, excluded) => {
                Rewrite::Exclusion(Box::new(base.try_map(f)?), Box::new(excluded.try_map(f)?))
            }
        })
    }
}
