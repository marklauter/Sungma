//! A namespace's history gives each relation a version declares an epoch,
//! which the relation keeps for as long as each next version declares it,
//! and refuses an entry that doesn't come after the last.

use std::{collections::HashMap, sync::Arc};

use sungma::{
    id::RelationId,
    namespace::Namespace,
    revision::Revision,
    rewrite::Rewrite::This,
    version::{Entry, History, HistoryError, StoredVersionError, Version},
};

const OWNER: RelationId = RelationId(0);
const BANNED: RelationId = RelationId(1);

fn declaring(relations: &[RelationId]) -> Namespace {
    Namespace::new(relations.iter().map(|&relation| (relation, This)).collect()).unwrap()
}

fn written(history: &mut History, relations: &[RelationId], revision: u64) {
    history
        .write(declaring(relations), Revision(revision))
        .unwrap();
}

#[test]
fn a_first_version_starts_every_epoch() {
    let mut history = History::default();
    assert!(history.latest().is_none());
    written(&mut history, &[OWNER, BANNED], 10);
    let version = history.latest().unwrap();
    assert_eq!(version.revision(), Revision(10));
    assert_eq!(version.epoch(OWNER), Some(Revision(10)));
    assert_eq!(version.epoch(BANNED), Some(Revision(10)));
    assert!(version.namespace().rewrite(&OWNER).is_some());
}

/// The sample in `docs/specs/one-timeline.md`, without its facts.
#[test]
fn a_relation_brought_back_gets_a_new_epoch() {
    let mut history = History::default();
    written(&mut history, &[OWNER, BANNED], 10);
    written(&mut history, &[OWNER], 20);
    let dropped = history.latest().unwrap();
    assert_eq!(dropped.epoch(OWNER), Some(Revision(10)));
    assert_eq!(dropped.epoch(BANNED), None);
    written(&mut history, &[OWNER, BANNED], 30);
    let back = history.latest().unwrap();
    assert_eq!(back.epoch(OWNER), Some(Revision(10)));
    assert_eq!(back.epoch(BANNED), Some(Revision(30)));
    history.retire(Revision(40)).unwrap();
    assert!(history.latest().is_none());
    written(&mut history, &[OWNER, BANNED], 50);
    let again = history.latest().unwrap();
    assert_eq!(again.epoch(OWNER), Some(Revision(50)));
    assert_eq!(again.epoch(BANNED), Some(Revision(50)));
    let opens: Vec<_> = history.entries().iter().map(Entry::revision).collect();
    assert_eq!(opens, [10, 20, 30, 40, 50].map(Revision));
    assert!(matches!(history.entries()[3], Entry::Retired(_)));
}

#[test]
fn an_entry_must_come_after_the_last() {
    let mut history = History::default();
    written(&mut history, &[OWNER], 10);
    for revision in [Revision(10), Revision(9)] {
        let refused = HistoryError::NotLater {
            last: Revision(10),
            revision,
        };
        assert_eq!(
            history.write(declaring(&[OWNER]), revision),
            Err(refused.clone())
        );
        assert_eq!(history.retire(revision), Err(refused));
    }
    assert_eq!(history.entries().len(), 1);
    assert_eq!(
        HistoryError::NotLater {
            last: Revision(10),
            revision: Revision(9),
        }
        .to_string(),
        "revision Revision(9) doesn't come after Revision(10)"
    );
}

#[test]
fn only_a_current_version_can_be_retired() {
    let mut history = History::default();
    assert_eq!(
        history.retire(Revision(1)),
        Err(HistoryError::NothingToRetire)
    );
    written(&mut history, &[OWNER], 2);
    history.retire(Revision(3)).unwrap();
    assert_eq!(
        history.retire(Revision(4)),
        Err(HistoryError::NothingToRetire)
    );
    assert_eq!(
        HistoryError::NothingToRetire.to_string(),
        "the namespace has no latest version to retire"
    );
}

fn stored(
    relations: &[RelationId],
    epochs: &[(RelationId, u64)],
) -> Result<Version, StoredVersionError> {
    let epochs = epochs
        .iter()
        .map(|&(relation, epoch)| (relation, Revision(epoch)))
        .collect::<HashMap<_, _>>();
    Version::stored(declaring(relations), Revision(30), epochs)
}

#[test]
fn a_stored_version_keeps_its_epochs() {
    let version = stored(&[OWNER, BANNED], &[(OWNER, 10), (BANNED, 30)]).unwrap();
    assert_eq!(version.revision(), Revision(30));
    assert_eq!(version.epoch(OWNER), Some(Revision(10)));
    assert_eq!(version.epoch(BANNED), Some(Revision(30)));
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

/// A history resumed from a stored version carries its epochs on and
/// refuses an entry that isn't later.
#[test]
fn a_resumed_history_goes_on_from_its_latest_entry() {
    let latest = Arc::new(stored(&[OWNER], &[(OWNER, 10)]).unwrap());
    let mut history = History::resume(Entry::Version(latest));
    assert_eq!(
        history.write(declaring(&[OWNER]), Revision(30)),
        Err(HistoryError::NotLater {
            last: Revision(30),
            revision: Revision(30)
        })
    );
    written(&mut history, &[OWNER, BANNED], 40);
    let next = history.latest().unwrap();
    assert_eq!(next.epoch(OWNER), Some(Revision(10)));
    assert_eq!(next.epoch(BANNED), Some(Revision(40)));
    let mut retired = History::resume(Entry::Retired(Revision(30)));
    assert_eq!(
        retired.retire(Revision(40)),
        Err(HistoryError::NothingToRetire)
    );
    written(&mut retired, &[OWNER], 40);
    assert_eq!(retired.latest().unwrap().epoch(OWNER), Some(Revision(40)));
}
