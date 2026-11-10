//! The write services over the in-memory store.

use sungma::{
    name::{FactName, NamespaceName, RelationName, SubjectName},
    namespace::{Constraint, Namespace, NamespaceError},
    revision::Revision,
    rewrite::Rewrite,
    store::StoreError,
};
use sungma_check::store::FactReader;
use sungma_clock::ClockFault;
use sungma_names::{resolve, store::Dictionary, store::Pool};
use sungma_node::testkit::World;
use sungma_write::{
    service::{
        FactWriteService, FactWriteServiceError, NamespaceWriteService, NamespaceWriteServiceError,
        SeedError, seed,
    },
    store::{FactWrite, FactWriteError, FactWriter, NamespaceWrite, NamespaceWriteError},
};

fn relations(names: &[&str]) -> Namespace<RelationName, NamespaceName> {
    Namespace::new(
        names
            .iter()
            .map(|name| {
                (
                    name.parse().unwrap(),
                    Rewrite::This,
                    vec![Constraint::Identity],
                )
            })
            .collect(),
    )
    .unwrap()
}

fn owner(identity: &str) -> FactName {
    FactName {
        subjectset: "file:readme#owner".parse().unwrap(),
        subject: SubjectName::Identity(identity.parse().unwrap()),
    }
}

/// `file` declaring `owner`, written through the service.
async fn declared(world: &World) -> Revision {
    NamespaceWriteService::new(world.node().interner(), world.node().writer())
        .write(
            &"file".parse().unwrap(),
            NamespaceWrite::Create(relations(&["owner"])),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn a_refused_insert_is_named_as_the_caller_named_it() {
    let world = World::new();
    declared(&world).await;
    world.intern(Pool::Namespaces, "folder").await;
    let undeclared = FactName {
        subjectset: "folder:root#owner".parse().unwrap(),
        subject: SubjectName::Identity("alice".parse().unwrap()),
    };
    let dictionary = world.names();
    let written = FactWriteService::new(dictionary, world.node().writer())
        .write(&[owner("alice"), undeclared.clone()], &[])
        .await;
    assert!(
        matches!(&written, Err(FactWriteServiceError::Undeclared(name)) if *name == undeclared),
        "{written:?}"
    );
}

/// A writer that fails as told.
struct Failing(fn() -> FactWriteError);

impl FactWriter for Failing {
    async fn write(&self, _: &[FactWrite]) -> Result<Revision, FactWriteError> {
        Err((self.0)())
    }
}

#[tokio::test]
async fn a_writer_that_fails_fails_the_write() {
    let world = World::new();
    declared(&world).await;
    let dictionary = world.names();
    let clock = Failing(|| ClockFault::Drifted.into());
    let written = FactWriteService::new(dictionary, &clock)
        .write(&[owner("alice")], &[])
        .await;
    assert!(
        matches!(
            written,
            Err(FactWriteServiceError::Clock(ClockFault::Drifted))
        ),
        "{written:?}"
    );
    let store = Failing(|| StoreError("down".to_owned()).into());
    let written = FactWriteService::new(dictionary, &store)
        .write(&[], &[owner("alice")])
        .await;
    assert!(
        matches!(written, Err(FactWriteServiceError::Store(_))),
        "{written:?}"
    );
    // A writer that refuses a fact it wasn't asked to add has failed.
    fn unasked() -> sungma::graph::Fact {
        sungma::graph::Fact {
            subjectset: sungma::graph::Subjectset {
                resource: sungma::graph::Resource {
                    namespace: sungma::id::NamespaceId(99),
                    id: "x".parse().unwrap(),
                },
                relation: sungma::id::RelationId(99),
            },
            subject: sungma::graph::Subject::Identity("x".parse().unwrap()),
        }
    }
    let confused = Failing(|| FactWriteError::Unsatisfied(unasked()));
    let written = FactWriteService::new(dictionary, &confused)
        .write(&[owner("alice")], &[])
        .await;
    assert!(
        matches!(written, Err(FactWriteServiceError::Store(_))),
        "{written:?}"
    );
    let confused = Failing(|| {
        FactWriteError::Undeclared(sungma::graph::Fact {
            subjectset: sungma::graph::Subjectset {
                resource: sungma::graph::Resource {
                    namespace: sungma::id::NamespaceId(99),
                    id: "x".parse().unwrap(),
                },
                relation: sungma::id::RelationId(99),
            },
            subject: sungma::graph::Subject::Identity("x".parse().unwrap()),
        })
    });
    let written = FactWriteService::new(dictionary, &confused)
        .write(&[owner("alice")], &[])
        .await;
    assert!(
        matches!(written, Err(FactWriteServiceError::Store(_))),
        "{written:?}"
    );
}

#[tokio::test]
async fn retiring_a_namespace_never_interned_interns_nothing() {
    let world = World::new();
    let retired = NamespaceWriteService::new(world.node().interner(), world.node().writer())
        .write(
            &"file".parse().unwrap(),
            NamespaceWrite::Retire {
                based_on: Revision(1),
            },
        )
        .await;
    assert!(
        matches!(
            retired,
            Err(NamespaceWriteServiceError::Write(
                NamespaceWriteError::NothingToRetire
            ))
        ),
        "{retired:?}"
    );
    assert_eq!(
        world
            .names()
            .lookup(Pool::Namespaces, "file")
            .await
            .unwrap(),
        None
    );
}

/// Names are interned before the write, so a conflict leaves them.
#[tokio::test]
async fn a_conflict_leaves_its_names_interned() {
    let world = World::new();
    let latest = declared(&world).await;
    let written = NamespaceWriteService::new(world.node().interner(), world.node().writer())
        .write(
            &"file".parse().unwrap(),
            NamespaceWrite::Create(relations(&["owner", "viewer"])),
        )
        .await;
    assert!(
        matches!(
            written,
            Err(NamespaceWriteServiceError::Write(NamespaceWriteError::Conflict {
                latest: Some(revision)
            })) if revision == latest
        ),
        "{written:?}"
    );
    assert!(
        world
            .names()
            .lookup(Pool::Relations, "viewer")
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn a_new_version_and_a_retirement_follow_the_latest_version() {
    let world = World::new();
    let first = declared(&world).await;
    let service = NamespaceWriteService::new(world.node().interner(), world.node().writer());
    let file = "file".parse().unwrap();
    let second = service
        .write(
            &file,
            NamespaceWrite::NewVersion {
                based_on: first,
                namespace: relations(&["owner", "viewer"]),
            },
        )
        .await
        .unwrap();
    assert!(second > first);
    let retired = service
        .write(&file, NamespaceWrite::Retire { based_on: second })
        .await
        .unwrap();
    assert!(retired > second);
    let id = sungma::id::NamespaceId(world.intern(Pool::Namespaces, "file").await);
    assert!(world.latest(id).await.is_none());
}

fn file() -> Vec<(NamespaceName, Namespace<RelationName, NamespaceName>)> {
    vec![("file".parse().unwrap(), relations(&["owner"]))]
}

/// A seed holds as many facts as it likes, whatever one write holds, and
/// takes a revision for each namespace and each write of facts.
#[tokio::test]
async fn a_large_seed_is_written_in_parts() {
    let world = World::new();
    let facts: Vec<FactName> = (0..250)
        .map(|i| FactName {
            subjectset: format!("file:f{i}#owner").parse().unwrap(),
            subject: SubjectName::Identity(format!("p{i}").parse().unwrap()),
        })
        .collect();
    let node = world.node();
    seed(node.interner(), node.writer(), file(), &facts)
        .await
        .unwrap();
    // One namespace, then 250 facts at 24 a write.
    assert_eq!(world.head().await, Revision(1 + 11));
    for i in [0, 249] {
        let set = format!("file:f{i}#owner").parse().unwrap();
        let set = resolve::subjectset(world.names(), &set).await.unwrap();
        let held = world.store().subjects(&set, world.head().await).await;
        assert_eq!(held.unwrap().len(), 1, "f{i}");
    }
}

#[tokio::test]
async fn a_seed_of_nothing_writes_nothing() {
    let world = World::new();
    let node = world.node();
    seed(node.interner(), node.writer(), Vec::new(), &[])
        .await
        .unwrap();
    assert_eq!(world.head().await, Revision(0));
}

/// A namespace that has a version can't be created again.
#[tokio::test]
async fn a_seed_into_a_store_with_the_namespace_is_refused() {
    let world = World::new();
    let node = world.node();
    seed(node.interner(), node.writer(), file(), &[])
        .await
        .unwrap();
    let seeded = seed(node.interner(), node.writer(), file(), &[]).await;
    assert!(matches!(seeded, Err(SeedError::Namespace(_))), "{seeded:?}");
}

/// A fact the namespaces don't declare refuses the facts, after the
/// namespaces were written.
#[tokio::test]
async fn facts_the_namespaces_do_not_declare_are_refused() {
    let world = World::new();
    let node = world.node();
    let editor = FactName {
        subjectset: "file:a#editor".parse().unwrap(),
        subject: SubjectName::Identity("alice".parse().unwrap()),
    };
    let seeded = seed(node.interner(), node.writer(), file(), &[editor]).await;
    assert!(matches!(seeded, Err(SeedError::Fact(_))), "{seeded:?}");
    assert_eq!(world.head().await, Revision(1));
}

/// A fact whose subject its relation's constraints don't admit is refused,
/// named as the caller named it.
#[tokio::test]
async fn a_fact_its_relation_does_not_admit_is_named_as_the_caller_named_it() {
    let world = World::new();
    declared(&world).await;
    world.intern(Pool::Namespaces, "folder").await;
    let folder = FactName {
        subjectset: "file:readme#owner".parse().unwrap(),
        subject: SubjectName::Resource("folder:root".parse().unwrap()),
    };
    let written = FactWriteService::new(world.names(), world.node().writer())
        .write(&[owner("alice"), folder.clone()], &[])
        .await;
    assert!(
        matches!(&written, Err(FactWriteServiceError::Unsatisfied(name)) if *name == folder),
        "{written:?}"
    );
    assert_eq!(
        written.unwrap_err().to_string(),
        "'file:readme#owner@folder:root' has a subject its relation's constraints don't admit"
    );
}

/// A namespace's constraints are interned and stored with it, and one that
/// names a relation another namespace doesn't declare is refused with its
/// names.
#[tokio::test]
async fn constraints_are_interned_and_checked_against_other_namespaces() {
    let world = World::new();
    let service = NamespaceWriteService::new(world.node().interner(), world.node().writer());
    let doc = |text: &str| sungma_lang::tdl::parse_one(text).unwrap().relations;
    let refused = service
        .write(
            &"file".parse().unwrap(),
            NamespaceWrite::Create(doc("namespace: file
  owner<group#member, folder>
")),
        )
        .await;
    let Err(NamespaceWriteServiceError::Write(NamespaceWriteError::References(problems))) = refused
    else {
        panic!("expected refused references, got {refused:?}");
    };
    assert_eq!(
        problems
            .iter()
            .map(|problem| &problem.error)
            .collect::<Vec<_>>(),
        [&NamespaceError::UndeclaredConstraint {
            relation: "owner".to_owned(),
            constraint: "group#member".to_owned(),
        }]
    );
    service
        .write(
            &"group".parse().unwrap(),
            NamespaceWrite::Create(doc("namespace: group\n  member<@identity, group#member>\n")),
        )
        .await
        .unwrap();
    service
        .write(
            &"file".parse().unwrap(),
            NamespaceWrite::Create(doc("namespace: file
  owner<group#member, folder>
")),
        )
        .await
        .unwrap();
    let folder = resolve::namespace(world.names(), &"folder".parse().unwrap()).await;
    let folder = folder.expect("the constraint's namespace is interned");
    let file = resolve::namespace(world.names(), &"file".parse().unwrap())
        .await
        .unwrap();
    let owner = resolve::relation(world.names(), &"owner".parse().unwrap())
        .await
        .unwrap();
    let version = world.latest(file).await.unwrap();
    assert!(
        version
            .namespace()
            .constraints(&owner)
            .contains(&Constraint::Resource(folder))
    );
    let lookup = world
        .names()
        .lookup(Pool::Namespaces, "group")
        .await
        .unwrap();
    assert!(lookup.is_some());
}
