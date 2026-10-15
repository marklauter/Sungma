//! The rules every [`FactWriter`](crate::store::FactWriter) applies to an
//! insert, kept here so each store applies the same ones. See
//! `docs/specs/one-timeline.md`.

use sungma::{
    graph::{Fact, Subject, Subjectset},
    id::NamespaceId,
    revision::Revision,
    version::Version,
};

use crate::store::{FactWrite, FactWriteError};

/// Refuses the first insert in `writes` whose relation, or whose
/// subjectset subject's relation, the current version doesn't declare.
/// `current` gives a namespace's current version, `None` when it has none.
pub fn check_inserts<'v>(
    writes: &[FactWrite],
    current: impl Fn(NamespaceId) -> Option<&'v Version>,
) -> Result<(), FactWriteError> {
    let declares = |set: &Subjectset| {
        current(set.resource.namespace).is_some_and(|version| version.declares(set.relation))
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
/// wrote the fact's open span, `None` when it has none, and `current` is
/// its namespace's current version. An open span written before its
/// relation's epoch doesn't count, so it closes and a new one opens.
pub fn opens_span(fact: &Fact, open: Option<Revision>, current: Option<&Version>) -> bool {
    open.is_none_or(|written| {
        !current.is_some_and(|version| version.counts(fact.subjectset.relation, written))
    })
}
