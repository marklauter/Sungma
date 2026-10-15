//! The sample in `docs/specs/one-timeline.md`, judged by the closure: facts
//! and namespace versions on one timeline, read at each revision.

use sungma::{
    closure::{Closure, Expansion},
    graph::{Fact, Resource, Subject, Subjectset},
    id::{NamespaceId, RelationId},
    name::{Identity, ResourceId},
    namespace::Namespace,
    revision::Revision,
    rewrite::Rewrite::{self, FactTo, This},
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

const GROUP: NamespaceId = NamespaceId(1);
const VIEWER: RelationId = RelationId(2);
const PARENT: RelationId = RelationId(3);
const INHERITED: RelationId = RelationId(4);
const MEMBER: RelationId = RelationId(5);

fn resource(namespace: NamespaceId, id: &str) -> Resource {
    Resource {
        namespace,
        id: ResourceId::new_unchecked(id),
    }
}

fn at(resource: Resource, relation: RelationId) -> Subjectset {
    Subjectset { resource, relation }
}

/// `viewer` and `parent` hold facts, and `inherited` is the viewers of
/// each parent.
fn file(relations: &[RelationId]) -> Namespace {
    let rule = |relation| match relation {
        INHERITED => FactTo {
            factset: PARENT,
            computed: VIEWER,
        },
        _ => This,
    };
    let relations: Vec<(RelationId, Rewrite)> = relations
        .iter()
        .map(|&relation| (relation, rule(relation)))
        .collect();
    Namespace::new(relations).unwrap()
}

/// Whether Alice, a member of `group:eng`, views `file:a` through the
/// nested fact `file:a#viewer@group:eng#member`, and whether Bob, a viewer
/// of `file:root`, inherits `file:a` through the fact
/// `file:a#parent@file:root`. The expansion of `file:a#viewer` lists the
/// facts that count under it.
async fn reached(store: &MemoryStore, revision: Revision) -> (bool, bool, Expansion) {
    let viewer = at(resource(FILE, "a"), VIEWER);
    let inherited = at(resource(FILE, "a"), INHERITED);
    let alice = Closure::new(store, store, &viewer, revision)
        .contains(&person("alice"))
        .await
        .unwrap();
    let bob = Closure::new(store, store, &inherited, revision)
        .contains(&person("bob"))
        .await
        .unwrap();
    let expanded = Closure::new(store, store, &viewer, revision)
        .expand()
        .await
        .unwrap()
        .unwrap();
    (alice, bob, expanded)
}

/// A nested subjectset and a parent link written before their relation's
/// epoch don't count, though the facts they lead to do.
#[tokio::test]
async fn a_path_through_a_fact_from_before_its_epoch_is_closed() {
    let store = MemoryStore::default();
    store
        .write_namespace(GROUP, Create(declaring(&[MEMBER])))
        .await
        .unwrap();
    let declared = file(&[VIEWER, PARENT, INHERITED]);
    let first = store.write_namespace(FILE, Create(declared)).await.unwrap();
    let eng = Subject::Subjectset(at(resource(GROUP, "eng"), MEMBER));
    let nested = Fact {
        subjectset: at(resource(FILE, "a"), VIEWER),
        subject: eng.clone(),
    };
    let parent = Fact {
        subjectset: at(resource(FILE, "a"), PARENT),
        subject: Subject::ResourceMember(resource(FILE, "root")),
    };
    let grants = [
        Insert(Fact {
            subjectset: at(resource(GROUP, "eng"), MEMBER),
            subject: person("alice"),
        }),
        Insert(nested.clone()),
        Insert(parent.clone()),
        Insert(Fact {
            subjectset: at(resource(FILE, "root"), VIEWER),
            subject: person("bob"),
        }),
    ];
    let granted = store.write(&grants).await.unwrap();
    let (alice, bob, expanded) = reached(&store, granted).await;
    assert!(alice && bob);
    assert_eq!(expanded, Expansion::Subjects(vec![eng.clone()]));

    // `viewer` and `parent` leave and come back, so their epochs move past
    // every fact above. Bob's viewer fact on the root is written again, so
    // only the parent link is from before the epoch.
    let dropped = store
        .write_namespace(
            FILE,
            NewVersion {
                based_on: first,
                namespace: declaring(&[OWNER]),
            },
        )
        .await
        .unwrap();
    let back = file(&[VIEWER, PARENT, INHERITED]);
    store
        .write_namespace(
            FILE,
            NewVersion {
                based_on: dropped,
                namespace: back,
            },
        )
        .await
        .unwrap();
    let root = Fact {
        subjectset: at(resource(FILE, "root"), VIEWER),
        subject: person("bob"),
    };
    let rewritten = store.write(&[Insert(root)]).await.unwrap();
    let (alice, bob, expanded) = reached(&store, rewritten).await;
    assert!(!alice && !bob);
    assert_eq!(expanded, Expansion::Subjects(vec![]));

    let again = store
        .write(&[Insert(nested), Insert(parent)])
        .await
        .unwrap();
    let (alice, bob, _) = reached(&store, again).await;
    assert!(alice && bob);
}
