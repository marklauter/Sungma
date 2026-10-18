//! The closure of a subjectset: the set of subjects derivable for it from
//! the facts under the namespaces, fixed at one revision.
//!
//! A closure is never built. [`Closure::decide`] judges one membership,
//! answering at each rewrite node and stopping as soon as the answer is
//! known, and cites the facts that establish an allowed membership.
//! [`Closure::contains`] walks the same way without collecting the facts.
//! [`Closure::expand`] materializes one level of the rewrite tree, leaving
//! referenced subjectsets as leaves. Check and Expand each have a module
//! of their own; both walk the same rewrites over the same reads.
//!
//! Namespaces are read at the same revision as facts. A subjectset whose
//! namespace or relation isn't declared then has no rewrite, and its
//! closure is empty. A fact counts only when its namespace's version
//! declares its relation and the fact was written at or after the
//! relation's epoch; see `docs/specs/one-timeline.md`.
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
//! [`ClosureError::DepthExceeded`], whatever the operands not yet evaluated
//! would give.

mod check;
mod expand;

use std::{future::Future, pin::Pin, sync::Arc};

use sungma::{
    graph::{Subject, Subjectset},
    id::RelationId,
    revision::Revision,
    rewrite::Rewrite,
    store::StoreError,
    version::Version,
};
use thiserror::Error;

use crate::store::{FactReader, NamespaceReader};

pub use expand::Expansion;

/// How many subjectsets one membership check may pass through without
/// repeating one.
pub const MAX_DEPTH: usize = 100;

#[derive(Debug, Error)]
pub enum ClosureError {
    #[error("membership check passed through more than {0} subjectsets")]
    DepthExceeded(usize),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// A recursive `async fn` has a future of infinite size, so the recursive
/// steps return their futures boxed.
pub(super) type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The items of facts stored under `relation` that count under `version`.
pub(super) fn counted<'a, T: 'a>(
    stored: Vec<(T, Revision)>,
    version: &'a Version,
    relation: RelationId,
) -> impl Iterator<Item = T> + 'a {
    stored
        .into_iter()
        .filter(move |(_, written)| version.counts(relation, *written))
        .map(|(item, _)| item)
}

/// `version` with the rewrite it gives `set`'s relation, `None` when it
/// doesn't declare the relation.
pub(super) fn ruled(
    version: Arc<Version>,
    set: &Subjectset,
) -> Option<(Arc<Version>, Arc<Rewrite>)> {
    let rewrite = Arc::clone(version.namespace().rewrite(&set.relation)?);
    Some((version, rewrite))
}

pub struct Closure<'a, T, F> {
    namespaces: &'a T,
    facts: &'a F,
    subjectset: &'a Subjectset,
    revision: Revision,
}

impl<'a, T: NamespaceReader + Sync, F: FactReader + Sync> Closure<'a, T, F> {
    pub fn new(
        namespaces: &'a T,
        facts: &'a F,
        subjectset: &'a Subjectset,
        revision: Revision,
    ) -> Self {
        Self {
            namespaces,
            facts,
            subjectset,
            revision,
        }
    }

    /// The version of `set`'s namespace at the revision, and the rewrite it
    /// gives `set`'s relation. `None` when the namespace or the relation
    /// isn't declared then.
    pub(super) async fn rule(
        &self,
        set: &Subjectset,
    ) -> Result<Option<(Arc<Version>, Arc<Rewrite>)>, StoreError> {
        let version = self
            .namespaces
            .version(set.resource.namespace, self.revision)
            .await?;
        Ok(version.and_then(|version| ruled(version, set)))
    }

    /// Each subject stored under `factset` that counts and names a resource,
    /// paired with that resource's `computed` subjectset. A resource member names its
    /// resource and a subjectset names the resource it belongs to, as in
    /// Zanzibar; identities name no resource and are skipped.
    pub(super) async fn fact_targets(
        &self,
        factset: &Subjectset,
        version: &Version,
        computed: RelationId,
    ) -> Result<Vec<(Subject, Subjectset)>, StoreError> {
        let subjects = self.facts.subjects(factset, self.revision).await?;
        Ok(counted(subjects, version, factset.relation)
            .filter_map(|subject| {
                let resource = match &subject {
                    Subject::ResourceMember(resource)
                    | Subject::Subjectset(Subjectset { resource, .. }) => resource.clone(),
                    Subject::Identity(_) => return None,
                };
                let target = Subjectset {
                    resource,
                    relation: computed,
                };
                Some((subject, target))
            })
            .collect())
    }
}
