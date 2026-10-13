//! The sample in `docs/specs/one-timeline.md`, judged by the closure: facts
//! and namespace versions on one timeline, read at each revision.

use sungma::{
    closure::Closure,
    graph::{Fact, Resource, Subject, Subjectset},
    id::{NamespaceId, RelationId},
    name::{Identity, ResourceId},
    namespace::Namespace,
    revision::Revision,
    rewrite::Rewrite::This,
};
use sungma_memory::MemoryStore;
use sungma_write::store::{
    FactWrite::Insert,
    FactWriter,
    NamespaceWrite::{Create, NewVersion, Retire},
    NamespaceWriter,
};

const FILE: NamespaceId = NamespaceId(0);
const OWNER: RelationId = RelationId(0);
const BANNED: RelationId = RelationId(1);

fn declaring(relations: &[RelationId]) -> Namespace {
    Namespace::new(relations.iter().map(|&relation| (relation, This)).collect()).unwrap()
}

fn set(relation: RelationId) -> Subjectset {
    Subjectset {
        resource: Resource {
            namespace: FILE,
            id: ResourceId::new_unchecked("a"),
        },
        relation,
    }
}

fn person(name: &str) -> Subject {
    Subject::Identity(Identity::new_unchecked(name))
}

async fn grant(store: &MemoryStore, relation: RelationId, who: &str) -> Revision {
    let fact = Fact {
        subjectset: set(relation),
        subject: person(who),
    };
    store.write(&[Insert(fact)]).await.unwrap()
}

/// Whether each of `owner@carol`, `banned@bob` and `banned@erin` counts.
async fn counted(store: &MemoryStore, revision: Revision) -> [bool; 3] {
    let mut counts = [false; 3];
    for (count, (relation, who)) in
        counts
            .iter_mut()
            .zip([(OWNER, "carol"), (BANNED, "bob"), (BANNED, "erin")])
    {
        let set = set(relation);
        let closure = Closure::new(store, store, &set, revision);
        *count = closure.contains(&person(who)).await.unwrap();
    }
    counts
}

/// The sample's revisions are 10, 12, 14 and so on; the store's clock
/// counts 1, 2, 3, in the same order.
#[tokio::test]
async fn the_one_timeline_sample() {
    let store = MemoryStore::default();
    let version_1 = store
        .write_namespace(FILE, Create(declaring(&[OWNER, BANNED])))
        .await
        .unwrap();
    grant(&store, OWNER, "carol").await;
    let bob = grant(&store, BANNED, "bob").await;
    let version_2 = store
        .write_namespace(
            FILE,
            NewVersion {
                based_on: version_1,
                namespace: declaring(&[OWNER]),
            },
        )
        .await
        .unwrap();
    let version_3 = store
        .write_namespace(
            FILE,
            NewVersion {
                based_on: version_2,
                namespace: declaring(&[OWNER, BANNED]),
            },
        )
        .await
        .unwrap();
    let erin = grant(&store, BANNED, "erin").await;
    let retired = store
        .write_namespace(
            FILE,
            Retire {
                based_on: version_3,
            },
        )
        .await
        .unwrap();
    let version_4 = store
        .write_namespace(FILE, Create(declaring(&[OWNER, BANNED])))
        .await
        .unwrap();
    let bob_again = grant(&store, BANNED, "bob").await;

    // Read at 15: version 1, and both facts were written after epoch 10.
    assert_eq!(counted(&store, bob).await, [true, true, false]);
    // Read at 25: version 2 doesn't declare `banned`.
    assert_eq!(counted(&store, version_2).await, [true, false, false]);
    // Read at 35: `banned` has epoch 30, after Bob's fact but not Erin's.
    assert_eq!(counted(&store, erin).await, [true, false, true]);
    // Read at 45: retired, so no version declares anything.
    assert_eq!(counted(&store, retired).await, [false; 3]);
    // Read at 55: every epoch is 50, and every fact is older.
    assert_eq!(counted(&store, version_4).await, [false; 3]);
    // Read at 65: Bob's fact, written again, opened a new span after 50.
    assert_eq!(counted(&store, bob_again).await, [false, true, false]);
}
