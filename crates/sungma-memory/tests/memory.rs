//! The in-memory fact store through its write port: facts keep their
//! history, so a read at any revision sees the facts stored then.

use std::collections::HashSet;

use proptest::{collection::vec, prelude::*, test_runner::TestCaseError};
use sungma::{
    graph::{Fact, Resource, Subject, Subjectset},
    id::{NamespaceId, RelationId},
    name::{Identity, ResourceId},
    revision::Revision,
    store::FactStore,
};
use sungma_memory::MemoryFactStore;
use sungma_write::store::{
    FactWrite::{self, *},
    FactWriter,
};

fn set() -> Subjectset {
    Subjectset {
        resource: Resource {
            namespace: NamespaceId(0),
            id: ResourceId::new_unchecked("doc"),
        },
        relation: RelationId(2),
    }
}

fn person(name: &str) -> Subject {
    Subject::Identity(Identity::new_unchecked(name))
}

fn alice() -> Subject {
    person("alice")
}

fn bob() -> Subject {
    person("bob")
}

fn fact(subject: Subject) -> Fact {
    Fact {
        subjectset: set(),
        subject,
    }
}

async fn stored(facts: &MemoryFactStore, revision: u64) -> Vec<Subject> {
    facts.subjects(&set(), Revision(revision)).await.unwrap()
}

#[tokio::test]
async fn each_write_is_the_next_revision() {
    let facts = MemoryFactStore::default();
    assert_eq!(
        facts.write(&[Insert(fact(alice()))]).await.unwrap(),
        Revision(1)
    );
    assert_eq!(facts.write(&[]).await.unwrap(), Revision(2));
    assert_eq!(facts.head().await.unwrap(), Revision(2));
}

#[tokio::test]
async fn a_batch_lands_at_one_revision() {
    let facts = MemoryFactStore::default();
    let revision = facts
        .write(&[Insert(fact(alice())), Insert(fact(bob()))])
        .await
        .unwrap();
    assert_eq!(revision, Revision(1));
    assert_eq!(stored(&facts, 0).await, []);
    assert_eq!(stored(&facts, 1).await, [alice(), bob()]);
}

#[tokio::test]
async fn a_deleted_fact_stays_visible_before_its_deletion() {
    let facts = MemoryFactStore::default();
    facts.write(&[Insert(fact(alice()))]).await.unwrap();
    facts.write(&[Delete(fact(alice()))]).await.unwrap();
    assert!(facts.contains(&set(), &alice(), Revision(1)).await.unwrap());
    assert!(!facts.contains(&set(), &alice(), Revision(2)).await.unwrap());
}

#[tokio::test]
async fn a_fact_stored_again_is_visible_in_each_span() {
    let facts = MemoryFactStore::default();
    facts.write(&[Insert(fact(alice()))]).await.unwrap();
    facts.write(&[Delete(fact(alice()))]).await.unwrap();
    facts.write(&[Insert(fact(alice()))]).await.unwrap();
    let mut seen = Vec::new();
    for revision in 1..=3 {
        seen.push(
            facts
                .contains(&set(), &alice(), Revision(revision))
                .await
                .unwrap(),
        );
    }
    assert_eq!(seen, [true, false, true]);
}

#[tokio::test]
async fn inserting_a_stored_fact_keeps_its_revision() {
    let facts = MemoryFactStore::default();
    facts.write(&[Insert(fact(alice()))]).await.unwrap();
    facts.write(&[Insert(fact(alice()))]).await.unwrap();
    facts.write(&[Delete(fact(alice()))]).await.unwrap();
    assert!(facts.contains(&set(), &alice(), Revision(1)).await.unwrap());
    assert!(!facts.contains(&set(), &alice(), Revision(3)).await.unwrap());
}

#[tokio::test]
async fn deleting_an_absent_fact_changes_nothing() {
    let facts = MemoryFactStore::default();
    facts.write(&[Delete(fact(alice()))]).await.unwrap();
    facts.write(&[Insert(fact(alice()))]).await.unwrap();
    facts.write(&[Delete(fact(bob()))]).await.unwrap();
    assert_eq!(stored(&facts, 3).await, [alice()]);
}

/// A second subjectset, also stored as a subject.
fn other() -> Subjectset {
    Subjectset {
        relation: RelationId(5),
        ..set()
    }
}

fn sets() -> [Subjectset; 2] {
    [set(), other()]
}

fn subjects() -> [Subject; 3] {
    [alice(), bob(), Subject::Subjectset(other())]
}

/// `(insert, set, subject)`: an insert or a delete of the fact of
/// `sets()[set]` and `subjects()[subject]`.
type Write = (bool, usize, usize);

/// Writes `batches` and compares every read at every revision with a model
/// that applies each batch's writes in order to a set of facts.
async fn reads_match_the_history(batches: Vec<Vec<Write>>) -> Result<(), TestCaseError> {
    let facts = MemoryFactStore::default();
    let (sets, subjects) = (sets(), subjects());
    let mut states = vec![HashSet::new()];
    for (i, batch) in batches.iter().enumerate() {
        let writes: Vec<FactWrite> = batch
            .iter()
            .map(|&(insert, set, subject)| {
                let fact = Fact {
                    subjectset: sets[set].clone(),
                    subject: subjects[subject].clone(),
                };
                if insert { Insert(fact) } else { Delete(fact) }
            })
            .collect();
        let revision = facts.write(&writes).await.unwrap();
        prop_assert_eq!(revision, Revision(i as u64 + 1));
        let mut state = states[i].clone();
        for &(insert, set, subject) in batch {
            if insert {
                state.insert((set, subject));
            } else {
                state.remove(&(set, subject));
            }
        }
        states.push(state);
    }
    let head = Revision(batches.len() as u64);
    prop_assert_eq!(facts.head().await.unwrap(), head);
    for (revision, state) in states.iter().enumerate() {
        let revision = Revision(revision as u64);
        for (i, set) in sets.iter().enumerate() {
            let expected: HashSet<Subject> = state
                .iter()
                .filter(|&&(stored, _)| stored == i)
                .map(|&(_, subject)| subjects[subject].clone())
                .collect();
            let stored = facts.subjects(set, revision).await.unwrap();
            prop_assert_eq!(stored.len(), expected.len(), "{:?}", revision);
            prop_assert_eq!(stored.into_iter().collect::<HashSet<_>>(), expected.clone());
            let nested: HashSet<Subject> = facts
                .subjectsets(set, revision)
                .await
                .unwrap()
                .into_iter()
                .map(Subject::Subjectset)
                .collect();
            let expected_nested = expected
                .iter()
                .filter(|subject| matches!(subject, Subject::Subjectset(_)))
                .cloned()
                .collect();
            prop_assert_eq!(nested, expected_nested);
            for subject in &subjects {
                let contains = facts.contains(set, subject, revision).await.unwrap();
                prop_assert_eq!(contains, expected.contains(subject));
            }
        }
    }
    Ok(())
}

proptest! {
    /// A read at a revision sees exactly the facts that applying every
    /// batch up to it, in order, leaves stored.
    #[test]
    fn a_read_at_a_revision_sees_the_facts_stored_then(
        batches in vec(vec((any::<bool>(), 0..2usize, 0..3usize), 0..4), 0..10),
    ) {
        let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
        runtime.block_on(reads_match_the_history(batches))?;
    }
}
