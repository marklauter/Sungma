//! The in-memory fact store through its write port: facts keep their
//! history, so a read at any revision sees the facts stored then.

use sungma_core::{
    memory::MemoryFactStore,
    model::{
        Fact, IdentityId, RelationId, Resource, ResourceId, Revision, Subject, Subjectset, TheoryId,
    },
    store::{FactStore, FactWrite::*, FactWriter},
};

const SET: Subjectset = Subjectset {
    resource: Resource {
        theory: TheoryId(0),
        id: ResourceId(1),
    },
    relation: RelationId(2),
};

const ALICE: Subject = Subject::Identity(IdentityId(3));
const BOB: Subject = Subject::Identity(IdentityId(4));

const fn fact(subject: Subject) -> Fact {
    Fact {
        subjectset: SET,
        subject,
    }
}

async fn stored(facts: &MemoryFactStore, revision: u64) -> Vec<Subject> {
    facts.subjects(SET, Revision(revision)).await.unwrap()
}

#[tokio::test]
async fn each_write_is_the_next_revision() {
    let facts = MemoryFactStore::default();
    assert_eq!(
        facts.write(&[Insert(fact(ALICE))]).await.unwrap(),
        Revision(1)
    );
    assert_eq!(facts.write(&[]).await.unwrap(), Revision(2));
    assert_eq!(facts.head().await.unwrap(), Revision(2));
}

#[tokio::test]
async fn a_batch_lands_at_one_revision() {
    let facts = MemoryFactStore::default();
    let revision = facts
        .write(&[Insert(fact(ALICE)), Insert(fact(BOB))])
        .await
        .unwrap();
    assert_eq!(revision, Revision(1));
    assert_eq!(stored(&facts, 0).await, []);
    assert_eq!(stored(&facts, 1).await, [ALICE, BOB]);
}

#[tokio::test]
async fn a_deleted_fact_stays_visible_before_its_deletion() {
    let facts = MemoryFactStore::default();
    facts.write(&[Insert(fact(ALICE))]).await.unwrap();
    facts.write(&[Delete(fact(ALICE))]).await.unwrap();
    assert!(facts.contains(SET, ALICE, Revision(1)).await.unwrap());
    assert!(!facts.contains(SET, ALICE, Revision(2)).await.unwrap());
}

#[tokio::test]
async fn a_fact_stored_again_is_visible_in_each_span() {
    let facts = MemoryFactStore::default();
    facts.write(&[Insert(fact(ALICE))]).await.unwrap();
    facts.write(&[Delete(fact(ALICE))]).await.unwrap();
    facts.write(&[Insert(fact(ALICE))]).await.unwrap();
    let mut seen = Vec::new();
    for revision in 1..=3 {
        seen.push(
            facts
                .contains(SET, ALICE, Revision(revision))
                .await
                .unwrap(),
        );
    }
    assert_eq!(seen, [true, false, true]);
}

#[tokio::test]
async fn inserting_a_stored_fact_keeps_its_revision() {
    let facts = MemoryFactStore::default();
    facts.write(&[Insert(fact(ALICE))]).await.unwrap();
    facts.write(&[Insert(fact(ALICE))]).await.unwrap();
    facts.write(&[Delete(fact(ALICE))]).await.unwrap();
    assert!(facts.contains(SET, ALICE, Revision(1)).await.unwrap());
    assert!(!facts.contains(SET, ALICE, Revision(3)).await.unwrap());
}

#[tokio::test]
async fn deleting_an_absent_fact_changes_nothing() {
    let facts = MemoryFactStore::default();
    facts.write(&[Delete(fact(ALICE))]).await.unwrap();
    facts.write(&[Insert(fact(ALICE))]).await.unwrap();
    facts.write(&[Delete(fact(BOB))]).await.unwrap();
    assert_eq!(stored(&facts, 3).await, [ALICE]);
}
