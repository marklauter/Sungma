//! The rules every [`FactWriter`](crate::store::FactWriter) applies to an
//! insert, kept here so each store applies the same ones. See
//! `docs/specs/one-timeline.md`.

use sungma::{
    graph::{Fact, Subject, Subjectset},
    id::NamespaceId,
    revision::Revision,
    version::Epochs,
};

use crate::store::{FactWrite, FactWriteError};

/// Refuses the first insert in `writes` whose relation, or whose
/// subjectset subject's relation, the latest version doesn't declare.
/// `latest` gives the epochs of a namespace's latest version, `None` when
/// it has none.
pub fn check_inserts<'v>(
    writes: &[FactWrite],
    latest: impl Fn(NamespaceId) -> Option<&'v Epochs>,
) -> Result<(), FactWriteError> {
    let declares = |set: &Subjectset| {
        latest(set.resource.namespace).is_some_and(|epochs| epochs.declares(set.relation))
    };
    for write in writes {
        let FactWrite::Insert(fact) = write else {
            continue;
        };
        let subject = match &fact.subject {
            Subject::Subjectset(subject) => declares(subject),
            Subject::Identity(_) | Subject::ResourceMember(_) => true,
        };
        if !(declares(&fact.subjectset) && subject) {
            return Err(FactWriteError::Undeclared(fact.clone()));
        }
    }
    Ok(())
}

/// Whether inserting `fact` opens a new span. `open` is the revision that
/// wrote the fact's open span, `None` when it has none, and `latest` is
/// the epochs of its namespace's latest version. An open span written
/// before its relation's epoch doesn't count, so it closes and a new one
/// opens.
pub fn opens_span(fact: &Fact, open: Option<Revision>, latest: Option<&Epochs>) -> bool {
    open.is_none_or(|written| {
        !latest.is_some_and(|epochs| epochs.counts(fact.subjectset.relation, written))
    })
}
