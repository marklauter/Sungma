//! The extent of a subjectset: the set of subjects derivable for it from
//! the facts under the theories, fixed at one revision.
//!
//! An extent is never built. [`Extent::contains`] judges one membership,
//! answering yes or no at each rewrite node and stopping as soon as the
//! answer is known. [`Extent::expand`] materializes one level of the
//! rewrite tree, leaving referenced subjectsets as leaves.
//!
//! A subjectset whose theory or relation isn't declared has no rewrite, and
//! its extent is empty.

use std::{future::Future, pin::Pin};

use thiserror::Error;

use crate::{
    model::{RelationId, Revision, Subject, Subjectset},
    rewrite::Rewrite,
    store::{FactStore, StoreError},
    theory::Theories,
};

/// How many subjectsets one membership check may pass through. Also stops
/// cycles.
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

pub struct Extent<'a, F> {
    theories: &'a Theories,
    facts: &'a F,
    subjectset: Subjectset,
    revision: Revision,
}

impl<'a, F: FactStore + Sync> Extent<'a, F> {
    pub fn new(
        theories: &'a Theories,
        facts: &'a F,
        subjectset: Subjectset,
        revision: Revision,
    ) -> Self {
        Self {
            theories,
            facts,
            subjectset,
            revision,
        }
    }

    /// Whether `subject` is in the extent.
    pub async fn contains(&self, subject: Subject) -> Result<bool, ExtentError> {
        self.contains_at(self.subjectset, subject, 0).await
    }

    /// One level of the rewrite tree, with the facts read at the revision.
    /// `None` when the subjectset has no rewrite.
    pub async fn expand(&self) -> Result<Option<Expansion>, ExtentError> {
        let Some(rewrite) = self.rewrite(self.subjectset) else {
            return Ok(None);
        };
        Ok(Some(self.expand_node(rewrite, self.subjectset).await?))
    }

    /// `None` when the theory or the relation isn't declared.
    fn rewrite(&self, set: Subjectset) -> Option<&'a Rewrite> {
        self.theories
            .get(&set.resource.theory)?
            .rewrite(set.relation)
    }

    fn contains_at(
        &self,
        set: Subjectset,
        subject: Subject,
        depth: usize,
    ) -> BoxFuture<'_, Result<bool, ExtentError>> {
        Box::pin(async move {
            if depth > MAX_DEPTH {
                return Err(ExtentError::DepthExceeded(MAX_DEPTH));
            }
            let Some(rewrite) = self.rewrite(set) else {
                return Ok(false);
            };
            self.contains_node(rewrite, set, subject, depth).await
        })
    }

    fn contains_node<'b>(
        &'b self,
        rewrite: &'b Rewrite,
        set: Subjectset,
        subject: Subject,
        depth: usize,
    ) -> BoxFuture<'b, Result<bool, ExtentError>> {
        Box::pin(async move {
            match rewrite {
                Rewrite::This => self.contains_this(set, subject, depth).await,
                Rewrite::Computed(relation) => {
                    let computed = Subjectset {
                        relation: *relation,
                        ..set
                    };
                    self.contains_at(computed, subject, depth + 1).await
                }
                Rewrite::FactTo { factset, computed } => {
                    for target in self.fact_targets(set, *factset, *computed).await? {
                        if self.contains_at(target, subject, depth + 1).await? {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                }
                Rewrite::Union(operands) => {
                    for operand in operands {
                        if self.contains_node(operand, set, subject, depth).await? {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                }
                Rewrite::Intersection(operands) => {
                    for operand in operands {
                        if !self.contains_node(operand, set, subject, depth).await? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                }
                Rewrite::Exclusion(base, excluded) => {
                    Ok(self.contains_node(base, set, subject, depth).await?
                        && !self.contains_node(excluded, set, subject, depth).await?)
                }
            }
        })
    }

    /// The subject is stored under `set` directly, or is a member of a
    /// subjectset stored there.
    async fn contains_this(
        &self,
        set: Subjectset,
        subject: Subject,
        depth: usize,
    ) -> Result<bool, ExtentError> {
        for fact_subject in self.facts.subjects(set, self.revision).await? {
            if fact_subject == subject {
                return Ok(true);
            }
            if let Subject::Subjectset(nested) = fact_subject
                && self.contains_at(nested, subject, depth + 1).await?
            {
                return Ok(true);
            }
        }
        Ok(false)
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
                        .map(Expansion::Reference)
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

    /// `(factset, computed)`: the `computed` subjectset of each resource
    /// named by a fact under `factset`. Only resource members name a
    /// resource; other subjects are skipped.
    async fn fact_targets(
        &self,
        set: Subjectset,
        factset: RelationId,
        computed: RelationId,
    ) -> Result<Vec<Subjectset>, ExtentError> {
        let factset = Subjectset {
            relation: factset,
            ..set
        };
        let subjects = self.facts.subjects(factset, self.revision).await?;
        Ok(subjects
            .into_iter()
            .filter_map(|subject| match subject {
                Subject::ResourceMember(resource) => Some(Subjectset {
                    resource,
                    relation: computed,
                }),
                _ => None,
            })
            .collect())
    }
}
