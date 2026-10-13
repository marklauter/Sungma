//! A namespace version gives each relation it declares an epoch, which a
//! relation keeps for as long as each next version declares it.

use sungma::{
    id::RelationId,
    namespace::Namespace,
    revision::Revision,
    rewrite::Rewrite::This,
    version::{NotLater, Version},
};

const OWNER: RelationId = RelationId(0);
const BANNED: RelationId = RelationId(1);

fn declaring(relations: &[RelationId]) -> Namespace {
    Namespace::new(relations.iter().map(|&relation| (relation, This)).collect()).unwrap()
}

#[test]
fn a_first_version_starts_every_epoch() {
    let version = Version::first(declaring(&[OWNER, BANNED]), Revision(10));
    assert_eq!(version.revision(), Revision(10));
    assert_eq!(version.epoch(OWNER), Some(Revision(10)));
    assert_eq!(version.epoch(BANNED), Some(Revision(10)));
    assert!(version.namespace().rewrite(&OWNER).is_some());
}

/// The sample in `docs/specs/one-timeline.md`, up to the retirement.
#[test]
fn a_relation_brought_back_gets_a_new_epoch() {
    let first = Version::first(declaring(&[OWNER, BANNED]), Revision(10));
    let dropped = first.next(declaring(&[OWNER]), Revision(20)).unwrap();
    assert_eq!(dropped.epoch(OWNER), Some(Revision(10)));
    assert_eq!(dropped.epoch(BANNED), None);
    let back = dropped
        .next(declaring(&[OWNER, BANNED]), Revision(30))
        .unwrap();
    assert_eq!(back.epoch(OWNER), Some(Revision(10)));
    assert_eq!(back.epoch(BANNED), Some(Revision(30)));
}

#[test]
fn a_version_must_come_after_the_one_it_follows() {
    let first = Version::first(declaring(&[OWNER]), Revision(10));
    for revision in [Revision(10), Revision(9)] {
        let refused = first.next(declaring(&[OWNER]), revision).unwrap_err();
        assert_eq!(
            refused,
            NotLater {
                current: Revision(10),
                next: revision,
            }
        );
    }
    assert_eq!(
        NotLater {
            current: Revision(10),
            next: Revision(9),
        }
        .to_string(),
        "a version at Revision(9) can't follow the version at Revision(10)"
    );
}
