//! Check: is a subject in a subjectset?
//!
//! Each rewrite node answers yes or no for the one subject in hand and
//! stops as soon as the answer is known, so no subject set is ever built.

use std::{future::Future, pin::Pin};

use thiserror::Error;

use crate::{
    model::{RelationId, Subject, Subjectset, TheoryId},
    rewrite::Rewrite,
    store::{FactStore, StoreError},
    theory::Theories,
};

/// How many subjectsets one check may pass through. Also stops cycles.
pub const MAX_DEPTH: usize = 100;

#[derive(Debug, Error)]
pub enum CheckError {
    #[error("theory {0:?} is not declared")]
    UndeclaredTheory(TheoryId),
    #[error("theory {theory:?} does not declare relation {relation:?}")]
    UndeclaredRelation {
        theory: TheoryId,
        relation: RelationId,
    },
    #[error("check passed through more than {0} subjectsets")]
    DepthExceeded(usize),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// A recursive `async fn` has a future of infinite size, so the recursive
/// steps return their futures boxed.
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub struct Checker<'a, F> {
    theories: &'a Theories,
    facts: &'a F,
}

impl<'a, F: FactStore + Sync> Checker<'a, F> {
    pub fn new(theories: &'a Theories, facts: &'a F) -> Self {
        Self { theories, facts }
    }

    pub async fn check(&self, set: Subjectset, subject: Subject) -> Result<bool, CheckError> {
        self.check_at(set, subject, 0).await
    }

    fn check_at(
        &self,
        set: Subjectset,
        subject: Subject,
        depth: usize,
    ) -> BoxFuture<'_, Result<bool, CheckError>> {
        Box::pin(async move {
            if depth > MAX_DEPTH {
                return Err(CheckError::DepthExceeded(MAX_DEPTH));
            }
            let theory = set.resource.theory;
            let rewrite = self
                .theories
                .get(&theory)
                .ok_or(CheckError::UndeclaredTheory(theory))?
                .rewrite(set.relation)
                .ok_or(CheckError::UndeclaredRelation {
                    theory,
                    relation: set.relation,
                })?;
            self.eval(rewrite, set, subject, depth).await
        })
    }

    fn eval<'b>(
        &'b self,
        rewrite: &'b Rewrite,
        set: Subjectset,
        subject: Subject,
        depth: usize,
    ) -> BoxFuture<'b, Result<bool, CheckError>> {
        Box::pin(async move {
            match rewrite {
                Rewrite::This => self.this(set, subject, depth).await,
                Rewrite::Computed(relation) => {
                    let computed = Subjectset {
                        relation: *relation,
                        ..set
                    };
                    self.check_at(computed, subject, depth + 1).await
                }
                Rewrite::FactTo { factset, computed } => {
                    let factset = Subjectset {
                        relation: *factset,
                        ..set
                    };
                    for fact_subject in self.facts.subjects(factset).await? {
                        // Only resource members name a resource to evaluate on.
                        if let Subject::ResourceMember(resource) = fact_subject {
                            let target = Subjectset {
                                resource,
                                relation: *computed,
                            };
                            if self.check_at(target, subject, depth + 1).await? {
                                return Ok(true);
                            }
                        }
                    }
                    Ok(false)
                }
                Rewrite::Union(operands) => {
                    for operand in operands {
                        if self.eval(operand, set, subject, depth).await? {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                }
                Rewrite::Intersection(operands) => {
                    for operand in operands {
                        if !self.eval(operand, set, subject, depth).await? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                }
                Rewrite::Exclusion(base, excluded) => {
                    Ok(self.eval(base, set, subject, depth).await?
                        && !self.eval(excluded, set, subject, depth).await?)
                }
            }
        })
    }

    /// The subject is stored under `set` directly, or is a member of a
    /// subjectset stored there.
    async fn this(
        &self,
        set: Subjectset,
        subject: Subject,
        depth: usize,
    ) -> Result<bool, CheckError> {
        for fact_subject in self.facts.subjects(set).await? {
            if fact_subject == subject {
                return Ok(true);
            }
            if let Subject::Subjectset(nested) = fact_subject
                && self.check_at(nested, subject, depth + 1).await?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}
