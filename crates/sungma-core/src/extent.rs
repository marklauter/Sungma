//! The extent of (closure over) a subjectset: the set of subjects derivable for it from
//! the facts under the theories, fixed at one revision.
//!
//! An extent is never built. [`Extent::decide`] judges one membership,
//! answering at each rewrite node and stopping as soon as the answer is
//! known, and cites the facts that establish an allowed membership.
//! [`Extent::contains`] walks the same way without collecting the facts.
//! [`Extent::expand`] materializes one level of the rewrite tree, leaving
//! referenced subjectsets as leaves.
//!
//! A subjectset whose theory or relation isn't declared has no rewrite, and
//! its extent is empty.
//!
//! Cycles end without an error. A membership check that reaches a
//! subjectset it is already evaluating finds nothing on that branch, and
//! the other branches go on. Evaluating a subjectset again from inside
//! itself would repeat the same steps forever, so the branch can't
//! contribute a derivation.
//!
//! Store errors follow Kleene's three-valued logic: a failed read is an
//! unknown verdict, and an operand that settles the result outweighs it. A
//! true union operand or a false intersection operand settles its
//! operator; a false base or a true excluded side settles an exclusion.
//! The verdict then doesn't depend on the order operands are evaluated in.
//! When nothing settles the result and a read failed, the check fails with
//! the first error met.
//!
//! The depth limit is not an unknown. It caps the work one check may do,
//! so passing it ends the whole check with
//! [`ExtentError::DepthExceeded`], whatever the operands not yet evaluated
//! would give.

use std::{future::Future, pin::Pin, sync::Arc};

use thiserror::Error;

use crate::{
    decision::{Decision, Outcome, SEMANTICS},
    model::{Fact, RelationId, Revision, Subject, Subjectset},
    rewrite::Rewrite,
    store::{FactStore, StoreError, TheoryStore},
};

/// How many subjectsets one membership check may pass through without
/// repeating one.
pub const MAX_DEPTH: usize = 100;

#[derive(Debug, Error)]
pub enum ExtentError {
    #[error("membership check passed through more than {0} subjectsets")]
    DepthExceeded(usize),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// One level of a subjectset's rewrite tree.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Expansion {
    /// `this`: the subjects stored directly. Subjectsets among them are
    /// references, not expanded.
    Subjects(Vec<Subject>),
    /// A subjectset to expand separately.
    Reference(Subjectset),
    Union(Vec<Expansion>),
    Intersection(Vec<Expansion>),
    Exclusion(Box<Expansion>, Box<Expansion>),
}

/// A recursive `async fn` has a future of infinite size, so the recursive
/// steps return their futures boxed.
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What a membership check collects as it succeeds: the facts that
/// establish it for [`Extent::decide`], or nothing for [`Extent::contains`].
trait Proof: Send + Sized {
    fn empty() -> Self;
    /// A fact stored under the subjectset in hand.
    fn fact(fact: Fact) -> Self;
    /// The fact that led to a subjectset, followed by the proof found there.
    fn cite(fact: Fact, proof: Self) -> Self;
    /// Adds the proof of another intersection operand.
    fn join(&mut self, proof: Self);
}

impl Proof for Vec<Fact> {
    fn empty() -> Self {
        Vec::new()
    }

    fn fact(fact: Fact) -> Self {
        vec![fact]
    }

    fn cite(fact: Fact, proof: Self) -> Self {
        let mut cited = Vec::with_capacity(proof.len() + 1);
        cited.push(fact);
        cited.extend(proof);
        cited
    }

    fn join(&mut self, proof: Self) {
        self.extend(proof);
    }
}

impl Proof for () {
    fn empty() -> Self {}

    fn fact(_: Fact) -> Self {}

    fn cite(_: Fact, _: Self) -> Self {}

    fn join(&mut self, _: Self) {}
}

/// One operand's answer in Kleene's three-valued logic.
enum Verdict<P> {
    Holds(P),
    Fails,
    /// A read failed, and nothing settled the answer.
    Unknown(StoreError),
}

use Verdict::{Fails, Holds, Unknown};

/// A step of a membership check. `Err` is the depth limit, which ends the
/// whole check.
type Walk<P> = Result<Verdict<P>, ExtentError>;

impl<P> Verdict<P> {
    fn map<Q>(self, f: impl FnOnce(P) -> Q) -> Verdict<Q> {
        match self {
            Holds(proof) => Holds(f(proof)),
            Fails => Fails,
            Unknown(error) => Unknown(error),
        }
    }

    /// Kleene's or: holds if either holds, else the first unknown.
    fn or(self, other: Self) -> Self {
        match (self, other) {
            (Holds(proof), _) | (_, Holds(proof)) => Holds(proof),
            (Unknown(error), _) | (_, Unknown(error)) => Unknown(error),
            (Fails, Fails) => Fails,
        }
    }

    /// Kleene's `self and not excluded`.
    fn unless<Q>(self, excluded: Verdict<Q>) -> Self {
        match (self, excluded) {
            (Fails, _) | (_, Holds(_)) => Fails,
            (Unknown(error), _) | (_, Unknown(error)) => Unknown(error),
            (Holds(proof), Fails) => Holds(proof),
        }
    }
}

impl<P: Proof> Verdict<P> {
    /// Kleene's and: fails if either fails, else the first unknown, else
    /// holds on both proofs.
    fn and(self, other: Self) -> Self {
        match (self, other) {
            (Fails, _) | (_, Fails) => Fails,
            (Unknown(error), _) | (_, Unknown(error)) => Unknown(error),
            (Holds(mut proof), Holds(other)) => {
                proof.join(other);
                Holds(proof)
            }
        }
    }
}

/// Kleene's or over `items`, judged in turn by `judge` until one holds.
async fn any<I, P, J>(items: impl IntoIterator<Item = I>, mut judge: impl FnMut(I) -> J) -> Walk<P>
where
    J: Future<Output = Walk<P>>,
{
    let mut answer = Fails;
    for item in items {
        answer = answer.or(judge(item).await?);
        if let Holds(_) = answer {
            break;
        }
    }
    Ok(answer)
}

pub struct Extent<'a, T, F> {
    theories: &'a T,
    facts: &'a F,
    subjectset: Subjectset,
    revision: Revision,
}

impl<'a, T: TheoryStore + Sync, F: FactStore + Sync> Extent<'a, T, F> {
    pub fn new(theories: &'a T, facts: &'a F, subjectset: Subjectset, revision: Revision) -> Self {
        Self {
            theories,
            facts,
            subjectset,
            revision,
        }
    }

    /// Judges whether `subject` is in the extent.
    pub async fn decide(&self, subject: Subject) -> Result<Decision, ExtentError> {
        let outcome = match self.judge(subject).await? {
            Some(grounds) => Outcome::Allowed { grounds },
            None => Outcome::Denied,
        };
        Ok(Decision {
            subjectset: self.subjectset,
            subject,
            revision: self.revision,
            semantics: SEMANTICS,
            outcome,
        })
    }

    /// Whether `subject` is in the extent. The fast path: the same walk as
    /// [`Extent::decide`], but it collects no grounds, so it allocates no
    /// facts. For callers that need many verdicts and no audit, such as
    /// filtering a list.
    pub async fn contains(&self, subject: Subject) -> Result<bool, ExtentError> {
        let proof: Option<()> = self.judge(subject).await?;
        Ok(proof.is_some())
    }

    /// The proof of membership, `None` when there is none.
    async fn judge<P: Proof>(&self, subject: Subject) -> Result<Option<P>, ExtentError> {
        match self.contains_at(self.subjectset, subject, &[]).await? {
            Holds(proof) => Ok(Some(proof)),
            Fails => Ok(None),
            Unknown(error) => Err(error.into()),
        }
    }

    /// One level of the rewrite tree, with the facts read at the revision.
    /// `None` when the subjectset has no rewrite.
    pub async fn expand(&self) -> Result<Option<Expansion>, ExtentError> {
        let Some(rewrite) = self.rewrite(self.subjectset).await? else {
            return Ok(None);
        };
        Ok(Some(self.expand_node(&rewrite, self.subjectset).await?))
    }

    /// `None` when the theory or the relation isn't declared.
    async fn rewrite(&self, set: Subjectset) -> Result<Option<Arc<Rewrite>>, StoreError> {
        self.theories
            .rewrite(set.resource.theory, set.relation)
            .await
    }

    /// `path` holds the subjectsets being evaluated, outermost first. The
    /// subject is the same throughout one check, so a subjectset on the
    /// path is a cycle.
    fn contains_at<'b, P: Proof + 'b>(
        &'b self,
        set: Subjectset,
        subject: Subject,
        path: &'b [Subjectset],
    ) -> BoxFuture<'b, Walk<P>> {
        Box::pin(async move {
            if path.contains(&set) {
                return Ok(Fails);
            }
            if path.len() >= MAX_DEPTH {
                return Err(ExtentError::DepthExceeded(MAX_DEPTH));
            }
            let rewrite = match self.rewrite(set).await {
                Ok(Some(rewrite)) => rewrite,
                Ok(None) => return Ok(Fails),
                Err(error) => return Ok(Unknown(error)),
            };
            let path = [path, &[set]].concat();
            self.contains_node(&rewrite, set, subject, &path).await
        })
    }

    fn contains_node<'b, P: Proof + 'b>(
        &'b self,
        rewrite: &'b Rewrite,
        set: Subjectset,
        subject: Subject,
        path: &'b [Subjectset],
    ) -> BoxFuture<'b, Walk<P>> {
        Box::pin(async move {
            match rewrite {
                Rewrite::This => self.contains_this(set, subject, path).await,
                Rewrite::Computed(relation) => {
                    let computed = Subjectset {
                        relation: *relation,
                        ..set
                    };
                    self.contains_at(computed, subject, path).await
                }
                Rewrite::FactTo { factset, computed } => {
                    let targets = match self.fact_targets(set, *factset, *computed).await {
                        Ok(targets) => targets,
                        Err(error) => return Ok(Unknown(error)),
                    };
                    any(targets, move |(fact, target)| async move {
                        let found = self.contains_at(target, subject, path).await?;
                        Ok(found.map(|proof| P::cite(fact, proof)))
                    })
                    .await
                }
                Rewrite::Union(operands) => {
                    any(operands, |operand| {
                        self.contains_node(operand, set, subject, path)
                    })
                    .await
                }
                Rewrite::Intersection(operands) => {
                    let mut answer = Holds(P::empty());
                    for operand in operands {
                        let found = self.contains_node(operand, set, subject, path).await?;
                        answer = answer.and(found);
                        if let Fails = answer {
                            break;
                        }
                    }
                    Ok(answer)
                }
                Rewrite::Exclusion(base, excluded) => {
                    let base = self.contains_node(base, set, subject, path).await?;
                    if let Fails = base {
                        return Ok(Fails);
                    }
                    // Only whether the excluded side holds matters.
                    let excluded: Verdict<()> =
                        self.contains_node(excluded, set, subject, path).await?;
                    Ok(base.unless(excluded))
                }
            }
        })
    }

    /// The subject is stored under `set` directly, found by a point lookup,
    /// or is a member of a subjectset stored there.
    async fn contains_this<P: Proof>(
        &self,
        set: Subjectset,
        subject: Subject,
        path: &[Subjectset],
    ) -> Walk<P> {
        let direct = match self.facts.contains(set, subject, self.revision).await {
            Ok(true) => {
                return Ok(Holds(P::fact(Fact {
                    subjectset: set,
                    subject,
                })));
            }
            Ok(false) => Fails,
            Err(error) => Unknown(error),
        };
        let nested = match self.facts.subjectsets(set, self.revision).await {
            Ok(nested) => {
                any(nested, move |nested| async move {
                    let fact = Fact {
                        subjectset: set,
                        subject: Subject::Subjectset(nested),
                    };
                    let found = self.contains_at(nested, subject, path).await?;
                    Ok(found.map(|proof| P::cite(fact, proof)))
                })
                .await?
            }
            Err(error) => Unknown(error),
        };
        Ok(direct.or(nested))
    }

    fn expand_node<'b>(
        &'b self,
        rewrite: &'b Rewrite,
        set: Subjectset,
    ) -> BoxFuture<'b, Result<Expansion, ExtentError>> {
        Box::pin(async move {
            Ok(match rewrite {
                Rewrite::This => {
                    Expansion::Subjects(self.facts.subjects(set, self.revision).await?)
                }
                Rewrite::Computed(relation) => Expansion::Reference(Subjectset {
                    relation: *relation,
                    ..set
                }),
                Rewrite::FactTo { factset, computed } => Expansion::Union(
                    self.fact_targets(set, *factset, *computed)
                        .await?
                        .into_iter()
                        .map(|(_, target)| Expansion::Reference(target))
                        .collect(),
                ),
                Rewrite::Union(operands) => Expansion::Union(self.expand_all(operands, set).await?),
                Rewrite::Intersection(operands) => {
                    Expansion::Intersection(self.expand_all(operands, set).await?)
                }
                Rewrite::Exclusion(base, excluded) => Expansion::Exclusion(
                    Box::new(self.expand_node(base, set).await?),
                    Box::new(self.expand_node(excluded, set).await?),
                ),
            })
        })
    }

    async fn expand_all(
        &self,
        operands: &[Rewrite],
        set: Subjectset,
    ) -> Result<Vec<Expansion>, ExtentError> {
        let mut expanded = Vec::with_capacity(operands.len());
        for operand in operands {
            expanded.push(self.expand_node(operand, set).await?);
        }
        Ok(expanded)
    }

    /// `(factset, computed)`: each fact under `factset` that names a
    /// resource, paired with that resource's `computed` subjectset. A
    /// resource member names its resource and a subjectset names the
    /// resource it belongs to, as in Zanzibar; identities name no resource
    /// and are skipped.
    async fn fact_targets(
        &self,
        set: Subjectset,
        factset: RelationId,
        computed: RelationId,
    ) -> Result<Vec<(Fact, Subjectset)>, StoreError> {
        let factset = Subjectset {
            relation: factset,
            ..set
        };
        let subjects = self.facts.subjects(factset, self.revision).await?;
        Ok(subjects
            .into_iter()
            .filter_map(|subject| match subject {
                Subject::ResourceMember(resource)
                | Subject::Subjectset(Subjectset { resource, .. }) => {
                    let fact = Fact {
                        subjectset: factset,
                        subject,
                    };
                    let target = Subjectset {
                        resource,
                        relation: computed,
                    };
                    Some((fact, target))
                }
                Subject::Identity(_) => None,
            })
            .collect())
    }
}
