//! Check: whether a subject is in the closure, and the facts that say so.

use std::{future::Future, sync::Arc};

use sungma::{
    graph::{Fact, Subject, Subjectset},
    non_empty::NonEmpty,
    rewrite::Rewrite,
    store::StoreError,
    version::Version,
};

use super::{BoxFuture, Closure, ClosureError, MAX_DEPTH, counted, ruled};
use crate::{
    decision::{Decision, Outcome, SEMANTICS},
    store::{FactReader, NamespaceReader},
};

/// The subjectsets being evaluated, innermost first. Each step links to the
/// one before it on the caller's stack, so a step costs no copy.
struct Path<'a> {
    set: Subjectset,
    up: Option<&'a Path<'a>>,
    depth: usize,
}

impl Path<'_> {
    fn holds(&self, set: &Subjectset) -> bool {
        let mut step = Some(self);
        while let Some(at) = step {
            if at.set == *set {
                return true;
            }
            step = at.up;
        }
        false
    }
}

/// What a membership check collects as it succeeds: the facts that
/// establish it for [`Closure::decide`], or nothing for [`Closure::contains`].
trait Proof: Send + Sized {
    /// A fact stored under the subjectset in hand, built only when the
    /// proof keeps it.
    fn fact(fact: impl FnOnce() -> Fact) -> Self;
    /// The fact that led to a subjectset, followed by the proof found there.
    /// The fact is built only when the proof keeps it.
    fn cite(fact: impl FnOnce() -> Fact, proof: Self) -> Self;
    /// Adds the proof of another intersection operand.
    fn join(&mut self, proof: Self);
}

impl Proof for NonEmpty<Fact> {
    fn fact(fact: impl FnOnce() -> Fact) -> Self {
        NonEmpty::one(fact())
    }

    fn cite(fact: impl FnOnce() -> Fact, proof: Self) -> Self {
        let mut cited = NonEmpty::one(fact());
        cited.append(proof);
        cited
    }

    fn join(&mut self, proof: Self) {
        self.append(proof);
    }
}

impl Proof for () {
    fn fact(_: impl FnOnce() -> Fact) -> Self {}

    fn cite(_: impl FnOnce() -> Fact, _: Self) -> Self {}

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
type Walk<P> = Result<Verdict<P>, ClosureError>;

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

impl<'a, T: NamespaceReader + Sync, F: FactReader + Sync> Closure<'a, T, F> {
    /// Judges whether `subject` is in the closure.
    pub async fn decide(&self, subject: Subject) -> Result<Decision, ClosureError> {
        let outcome = match self.judge(&subject).await? {
            Some(grounds) => Outcome::Allowed { grounds },
            None => Outcome::Denied,
        };
        Ok(Decision {
            subjectset: self.subjectset.clone(),
            subject,
            revision: self.revision,
            semantics: SEMANTICS,
            outcome,
        })
    }

    /// Whether `subject` is in the closure. The fast path: the same walk as
    /// [`Closure::decide`], but it collects no grounds, so it allocates no
    /// facts. For callers that need many verdicts and no audit, such as
    /// filtering a list.
    pub async fn contains(&self, subject: &Subject) -> Result<bool, ClosureError> {
        let proof: Option<()> = self.judge(subject).await?;
        Ok(proof.is_some())
    }

    /// The proof of membership, `None` when there is none.
    async fn judge<P: Proof>(&self, subject: &Subject) -> Result<Option<P>, ClosureError> {
        match self
            .contains_at(self.subjectset.clone(), None, subject, None)
            .await?
        {
            Holds(proof) => Ok(Some(proof)),
            Fails => Ok(None),
            Unknown(error) => Err(error.into()),
        }
    }

    /// `path` holds the subjectsets being evaluated, outermost first. The
    /// subject is the same throughout one check, so a subjectset on the
    /// path is a cycle. `known` is the version of `set`'s namespace when
    /// the caller already holds it, as a computed step on the same
    /// resource does, so it isn't read again.
    fn contains_at<'b, P: Proof + 'b>(
        &'b self,
        set: Subjectset,
        known: Option<&'b Arc<Version>>,
        subject: &'b Subject,
        path: Option<&'b Path<'b>>,
    ) -> BoxFuture<'b, Walk<P>> {
        Box::pin(async move {
            if path.is_some_and(|path| path.holds(&set)) {
                return Ok(Fails);
            }
            let depth = path.map_or(0, |path| path.depth);
            if depth >= MAX_DEPTH {
                return Err(ClosureError::DepthExceeded(MAX_DEPTH));
            }
            let rule = match known {
                Some(version) => Ok(ruled(Arc::clone(version), &set)),
                None => self.rule(&set).await,
            };
            let (version, rewrite) = match rule {
                Ok(Some(rule)) => rule,
                Ok(None) => return Ok(Fails),
                Err(error) => return Ok(Unknown(error)),
            };
            let path = Path {
                set,
                up: path,
                depth: depth + 1,
            };
            self.contains_node(&rewrite, &version, &path.set, subject, Some(&path))
                .await
        })
    }

    fn contains_node<'b, P: Proof + 'b>(
        &'b self,
        rewrite: &'b Rewrite,
        version: &'b Arc<Version>,
        set: &'b Subjectset,
        subject: &'b Subject,
        path: Option<&'b Path<'b>>,
    ) -> BoxFuture<'b, Walk<P>> {
        Box::pin(async move {
            match rewrite {
                Rewrite::This => self.contains_this(set, version, subject, path).await,
                Rewrite::Computed(relation) => {
                    let computed = Subjectset {
                        resource: set.resource.clone(),
                        relation: *relation,
                    };
                    self.contains_at(computed, Some(version), subject, path)
                        .await
                }
                Rewrite::FactTo { factset, computed } => {
                    let factset = &Subjectset {
                        resource: set.resource.clone(),
                        relation: *factset,
                    };
                    let targets = match self.fact_targets(factset, version, *computed).await {
                        Ok(targets) => targets,
                        Err(error) => return Ok(Unknown(error)),
                    };
                    any(targets, move |(cited, target)| async move {
                        let found = self.contains_at(target, None, subject, path).await?;
                        Ok(found.map(|proof| {
                            P::cite(
                                || Fact {
                                    subjectset: factset.clone(),
                                    subject: cited,
                                },
                                proof,
                            )
                        }))
                    })
                    .await
                }
                Rewrite::Union(operands) => {
                    any(operands, |operand| {
                        self.contains_node(operand, version, set, subject, path)
                    })
                    .await
                }
                Rewrite::Intersection(operands) => {
                    let (first, rest) = operands.first_and_rest();
                    let mut answer = self
                        .contains_node(first, version, set, subject, path)
                        .await?;
                    for operand in rest {
                        if let Fails = answer {
                            break;
                        }
                        let found = self
                            .contains_node(operand, version, set, subject, path)
                            .await?;
                        answer = answer.and(found);
                    }
                    Ok(answer)
                }
                Rewrite::Exclusion(base, excluded) => {
                    let base = self
                        .contains_node(base, version, set, subject, path)
                        .await?;
                    if let Fails = base {
                        return Ok(Fails);
                    }
                    // Only whether the excluded side holds matters.
                    let excluded: Verdict<()> = self
                        .contains_node(excluded, version, set, subject, path)
                        .await?;
                    Ok(base.unless(excluded))
                }
            }
        })
    }

    /// The subject is stored under `set` directly, found by a point lookup,
    /// or is a member of a subjectset stored there.
    async fn contains_this<P: Proof>(
        &self,
        set: &Subjectset,
        version: &Version,
        subject: &Subject,
        path: Option<&Path<'_>>,
    ) -> Walk<P> {
        let direct = match self.facts.written(set, subject, self.revision).await {
            Ok(Some(written)) if version.counts(set.relation, written) => {
                return Ok(Holds(P::fact(|| Fact {
                    subjectset: set.clone(),
                    subject: subject.clone(),
                })));
            }
            Ok(_) => Fails,
            Err(error) => Unknown(error),
        };
        let nested = match self.facts.subjectsets(set, self.revision).await {
            Ok(nested) => {
                any(
                    counted(nested, version, set.relation),
                    move |nested| async move {
                        let found = self
                            .contains_at(nested.clone(), None, subject, path)
                            .await?;
                        Ok(found.map(|proof| {
                            P::cite(
                                || Fact {
                                    subjectset: set.clone(),
                                    subject: Subject::Subjectset(nested),
                                },
                                proof,
                            )
                        }))
                    },
                )
                .await?
            }
            Err(error) => Unknown(error),
        };
        Ok(direct.or(nested))
    }
}
