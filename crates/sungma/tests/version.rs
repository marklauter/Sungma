//! A namespace version gives each relation it declares an epoch, which the
//! relation keeps for as long as each next version declares it.

use std::collections::HashMap;

use sungma::{
    id::RelationId,
    namespace::Namespace,
    revision::Revision,
    rewrite::Rewrite::This,
    version::{Epochs, StoredVersionError, Version},
};

const OWNER: RelationId = RelationId(0);
const BANNED: RelationId = RelationId(1);

fn declaring(relations: &[RelationId]) -> Namespace {
    Namespace::new(relations.iter().map(|&relation| (relation, This)).collect()).unwrap()
}

#[test]
fn a_first_version_starts_every_epoch() {
    let epochs = Epochs::following(None, [OWNER, BANNED], Revision(10));
    assert_eq!(epochs.epoch(OWNER), Some(Revision(10)));
    assert_eq!(epochs.epoch(BANNED), Some(Revision(10)));
    assert!(epochs.declares(OWNER));
}

/// The sample in `docs/specs/one-timeline.md`, without its facts.
#[test]
fn a_relation_brought_back_gets_a_new_epoch() {
    let first = Epochs::following(None, [OWNER, BANNED], Revision(10));
    let dropped = Epochs::following(Some(&first), [OWNER], Revision(20));
    assert_eq!(dropped.epoch(OWNER), Some(Revision(10)));
    assert_eq!(dropped.epoch(BANNED), None);
    let back = Epochs::following(Some(&dropped), [OWNER, BANNED], Revision(30));
    assert_eq!(back.epoch(OWNER), Some(Revision(10)));
    assert_eq!(back.epoch(BANNED), Some(Revision(30)));
    // A version after a retirement follows none.
    let again = Epochs::following(None, [OWNER, BANNED], Revision(50));
    assert_eq!(again.epoch(OWNER), Some(Revision(50)));
    assert_eq!(again.epoch(BANNED), Some(Revision(50)));
}

#[test]
fn a_fact_counts_from_its_relation_s_epoch() {
    let epochs = Epochs::following(None, [OWNER], Revision(10));
    assert!(!epochs.counts(OWNER, Revision(9)));
    assert!(epochs.counts(OWNER, Revision(10)));
    assert!(epochs.counts(OWNER, Revision(11)));
    assert!(!epochs.counts(BANNED, Revision(11)));
    let mut all: Vec<_> = epochs.iter().collect();
    all.sort();
    assert_eq!(all, [(OWNER, Revision(10))]);
}

fn stored(
    relations: &[RelationId],
    epochs: &[(RelationId, u64)],
) -> Result<Version, StoredVersionError> {
    let epochs = epochs
        .iter()
        .map(|&(relation, epoch)| (relation, Revision(epoch)))
        .collect::<HashMap<_, _>>();
    Version::stored(declaring(relations), Revision(30), Epochs::new(epochs))
}

#[test]
fn a_stored_version_keeps_its_epochs() {
    let version = stored(&[OWNER, BANNED], &[(OWNER, 10), (BANNED, 30)]).unwrap();
    assert_eq!(version.revision(), Revision(30));
    assert_eq!(version.epoch(OWNER), Some(Revision(10)));
    assert_eq!(version.epoch(BANNED), Some(Revision(30)));
    assert!(version.declares(OWNER));
    assert!(version.counts(BANNED, Revision(30)));
    assert!(!version.counts(BANNED, Revision(29)));
    assert!(version.namespace().rewrite(&OWNER).is_some());
}

#[test]
fn a_stored_version_needs_an_epoch_for_exactly_each_relation() {
    assert_eq!(
        stored(&[OWNER, BANNED], &[(OWNER, 10)]).unwrap_err(),
        StoredVersionError::NoEpoch(BANNED)
    );
    assert_eq!(
        stored(&[OWNER], &[(OWNER, 10), (BANNED, 10)]).unwrap_err(),
        StoredVersionError::Undeclared(BANNED)
    );
    assert_eq!(
        stored(&[OWNER], &[(OWNER, 31)]).unwrap_err(),
        StoredVersionError::LaterEpoch(OWNER)
    );
}
