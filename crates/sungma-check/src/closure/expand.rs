//! Expand: one level of a subjectset's rewrite tree, with the facts read
//! at the revision.

use sungma::{
    graph::{Subject, Subjectset},
    rewrite::Rewrite,
    version::Version,
};

use super::{BoxFuture, Closure, ClosureError, counted};
use crate::store::{FactReader, NamespaceReader};

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

impl<'a, T: NamespaceReader + Sync, F: FactReader + Sync> Closure<'a, T, F> {
    /// One level of the rewrite tree, with the facts read at the revision.
    /// `None` when the subjectset has no rewrite.
    pub async fn expand(&self) -> Result<Option<Expansion>, ClosureError> {
        let Some((version, rewrite)) = self.rule(self.subjectset).await? else {
            return Ok(None);
        };
        Ok(Some(
            self.expand_node(&rewrite, &version, self.subjectset)
                .await?,
        ))
    }

    fn expand_node<'b>(
        &'b self,
        rewrite: &'b Rewrite,
        version: &'b Version,
        set: &'b Subjectset,
    ) -> BoxFuture<'b, Result<Expansion, ClosureError>> {
        Box::pin(async move {
            Ok(match rewrite {
                Rewrite::This => {
                    let subjects = self.facts.subjects(set, self.revision).await?;
                    Expansion::Subjects(counted(subjects, version, set.relation).collect())
                }
                Rewrite::Computed(relation) => Expansion::Reference(Subjectset {
                    resource: set.resource.clone(),
                    relation: *relation,
                }),
                Rewrite::FactTo { factset, computed } => {
                    let factset = Subjectset {
                        resource: set.resource.clone(),
                        relation: *factset,
                    };
                    Expansion::Union(
                        self.fact_targets(&factset, version, *computed)
                            .await?
                            .into_iter()
                            .map(|(_, target)| Expansion::Reference(target))
                            .collect(),
                    )
                }
                Rewrite::Union(operands) => {
                    Expansion::Union(self.expand_all(operands, version, set).await?)
                }
                Rewrite::Intersection(operands) => {
                    Expansion::Intersection(self.expand_all(operands, version, set).await?)
                }
                Rewrite::Exclusion(base, excluded) => Expansion::Exclusion(
                    Box::new(self.expand_node(base, version, set).await?),
                    Box::new(self.expand_node(excluded, version, set).await?),
                ),
            })
        })
    }

    async fn expand_all(
        &self,
        operands: &[Rewrite],
        version: &Version,
        set: &Subjectset,
    ) -> Result<Vec<Expansion>, ClosureError> {
        let mut expanded = Vec::with_capacity(operands.len());
        for operand in operands {
            expanded.push(self.expand_node(operand, version, set).await?);
        }
        Ok(expanded)
    }
}
