//! The in-memory store's facts through the write port: facts keep their
//! history, so a read at any revision sees the facts stored then.

use std::collections::HashSet;

use proptest::{collection::vec, prelude::*, test_runner::TestCaseError};
use sungma::{
    graph::{Fact, Resource, Subject, Subjectset},
    id::{NamespaceId, RelationId},
    name::{Identity, ResourceId},
    namespace::Namespace,
    revision::Revision,
    rewrite::Rewrite::This,
    store::FactStore,
};
use sungma_memory::MemoryStore;
use sungma_write::store::{
    FactWrite::{self, *},
    FactWriteError, FactWriter,
};

const DOC: NamespaceId = NamespaceId(0);
const OWNER: RelationId = RelationId(2);
const MEMBER: RelationId = RelationId(5);

fn declaring(relations: &[RelationId]) -> Namespace {
    Namespace::new(relations.iter().map(|&relation| (relation, This)).collect()).unwrap()
}

/// A store whose namespace declares both relations, at revision 1.
fn store() -> MemoryStore {
    let store = MemoryStore::default();
    store.declare(DOC, declaring(&[OWNER, MEMBER])).unwrap();
    store
}

fn set() -> Subjectset {
    Subjectset {
        resource: Resource {
            namespace: DOC,
            id: ResourceId::new_unchecked("doc"),
        },
        relation: OWNER,
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

async fn stored(store: &MemoryStore, revision: u64) -> Vec<Subject> {
    store
        .subjects(&set(), Revision(revision))
        .await
        .unwrap()
        .into_iter()
        .map(|(subject, _)| subject)
        .collect()
}

async fn written(store: &MemoryStore, revision: u64) -> Option<Revision> {
    store
        .written(&set(), &alice(), Revision(revision))
        .await
        .unwrap()
}

#[tokio::test]
async fn each_write_is_the_next_revision() {
    let store = store();
    assert_eq!(
        store.write(&[Insert(fact(alice()))]).await.unwrap(),
        Revision(2)
    );
    assert_eq!(store.write(&[]).await.unwrap(), Revision(3));
    assert_eq!(store.head().await.unwrap(), Revision(3));
}

#[tokio::test]
async fn a_batch_lands_at_one_revision() {
    let store = store();
    let revision = store
        .write(&[Insert(fact(alice())), Insert(fact(bob()))])
        .await
        .unwrap();
    assert_eq!(revision, Revision(2));
    assert_eq!(stored(&store, 1).await, []);
    assert_eq!(stored(&store, 2).await, [alice(), bob()]);
}

#[tokio::test]
async fn a_deleted_fact_stays_visible_before_its_deletion() {
    let store = store();
    store.write(&[Insert(fact(alice()))]).await.unwrap();
    store.write(&[Delete(fact(alice()))]).await.unwrap();
    assert_eq!(written(&store, 2).await, Some(Revision(2)));
    assert_eq!(written(&store, 3).await, None);
}

#[tokio::test]
async fn a_fact_stored_again_is_visible_in_each_span() {
    let store = store();
    store.write(&[Insert(fact(alice()))]).await.unwrap();
    store.write(&[Delete(fact(alice()))]).await.unwrap();
    store.write(&[Insert(fact(alice()))]).await.unwrap();
    let mut seen = Vec::new();
    for revision in 2..=4 {
        seen.push(written(&store, revision).await);
    }
    assert_eq!(seen, [Some(Revision(2)), None, Some(Revision(4))]);
}

#[tokio::test]
async fn inserting_a_stored_fact_keeps_its_revision() {
    let store = store();
    store.write(&[Insert(fact(alice()))]).await.unwrap();
    store.write(&[Insert(fact(alice()))]).await.unwrap();
    assert_eq!(written(&store, 3).await, Some(Revision(2)));
    store.write(&[Delete(fact(alice()))]).await.unwrap();
    assert_eq!(written(&store, 4).await, None);
}

#[tokio::test]
async fn deleting_an_absent_fact_changes_nothing() {
    let store = store();
    store.write(&[Delete(fact(alice()))]).await.unwrap();
    store.write(&[Insert(fact(alice()))]).await.unwrap();
    store.write(&[Delete(fact(bob()))]).await.unwrap();
    assert_eq!(stored(&store, 4).await, [alice()]);
}

/// An insert the current versions don't declare is refused with the whole
/// batch, and takes no revision.
#[tokio::test]
async fn an_insert_under_an_undeclared_relation_stores_nothing() {
    let store = store();
    let undeclared = Subjectset {
        relation: RelationId(9),
        ..set()
    };
    let refusals = [
        Fact {
            subjectset: undeclared.clone(),
            subject: alice(),
        },
        fact(Subject::Subjectset(undeclared)),
        Fact {
            subjectset: Subjectset {
                resource: Resource {
                    namespace: NamespaceId(7),
                    id: ResourceId::new_unchecked("doc"),
                },
                relation: OWNER,
            },
            subject: alice(),
        },
    ];
    for refused in refusals {
        let result = store
            .write(&[Insert(fact(bob())), Insert(refused.clone())])
            .await;
        assert!(
            matches!(&result, Err(FactWriteError::Undeclared(fact)) if *fact == refused),
            "{result:?}"
        );
    }
    assert_eq!(store.head().await.unwrap(), Revision(1));
    assert_eq!(stored(&store, 1).await, []);
    let member = Subject::Subjectset(Subjectset {
        relation: MEMBER,
        ..set()
    });
    let resource = Subject::ResourceMember(set().resource);
    store
        .write(&[
            Insert(fact(member)),
            Insert(fact(resource)),
            Delete(fact(bob())),
        ])
        .await
        .unwrap();
}

/// A fact written before its relation's epoch is stored again in a new
/// span when it's inserted again, and one written after keeps its span.
#[tokio::test]
async fn inserting_a_fact_from_before_its_epoch_opens_a_new_span() {
    let store = store();
    store.write(&[Insert(fact(alice()))]).await.unwrap();
    store.declare(DOC, declaring(&[MEMBER])).unwrap();
    let back = store.declare(DOC, declaring(&[OWNER, MEMBER])).unwrap();
    assert_eq!(back, Revision(4));
    store.write(&[Insert(fact(alice()))]).await.unwrap();
    assert_eq!(written(&store, 4).await, Some(Revision(2)));
    assert_eq!(written(&store, 5).await, Some(Revision(5)));
    store.write(&[Insert(fact(alice()))]).await.unwrap();
    assert_eq!(written(&store, 6).await, Some(Revision(5)));
}

/// A second subjectset, also stored as a subject.
fn other() -> Subjectset {
    Subjectset {
        relation: MEMBER,
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
    let store = store();
    let first = store.head().await.unwrap().0;
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
        let revision = store.write(&writes).await.unwrap();
        prop_assert_eq!(revision, Revision(first + i as u64 + 1));
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
    let head = Revision(first + batches.len() as u64);
    prop_assert_eq!(store.head().await.unwrap(), head);
    for (i, state) in states.iter().enumerate() {
        let revision = Revision(first + i as u64);
        for (i, set) in sets.iter().enumerate() {
            let expected: HashSet<Subject> = state
                .iter()
                .filter(|&&(stored, _)| stored == i)
                .map(|&(_, subject)| subjects[subject].clone())
                .collect();
            let stored = store.subjects(set, revision).await.unwrap();
            prop_assert_eq!(stored.len(), expected.len(), "{:?}", revision);
            prop_assert!(stored.iter().all(|(_, written)| *written <= revision));
            let stored: HashSet<Subject> = stored.into_iter().map(|(subject, _)| subject).collect();
            prop_assert_eq!(stored, expected.clone());
            let nested: HashSet<Subject> = store
                .subjectsets(set, revision)
                .await
                .unwrap()
                .into_iter()
                .map(|(nested, _)| Subject::Subjectset(nested))
                .collect();
            let expected_nested = expected
                .iter()
                .filter(|subject| matches!(subject, Subject::Subjectset(_)))
                .cloned()
                .collect();
            prop_assert_eq!(nested, expected_nested);
            for subject in &subjects {
                let written = store.written(set, subject, revision).await.unwrap();
                prop_assert_eq!(written.is_some(), expected.contains(subject));
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
