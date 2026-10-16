//! The write services over the in-memory store.

use std::num::NonZeroU32;

use sungma::{
    name::{FactName, RelationName, SubjectName},
    namespace::Namespace,
    revision::Revision,
    rewrite::Rewrite,
    store::{Pool, ReadingDictionary, StoreError},
    version::HistoryError,
};
use sungma_clock::ClockFault;
use sungma_memory::{MemoryDictionary, MemoryStore};
use sungma_write::{
    intern::LeasingInterner,
    service::{
        FactWriteService, FactWriteServiceError, NamespaceWriteService, NamespaceWriteServiceError,
    },
    store::{FactWrite, FactWriteError, FactWriter, NamespaceWrite, NamespaceWriteError},
};

fn interner(names: &MemoryDictionary) -> LeasingInterner<&MemoryDictionary, &MemoryDictionary> {
    LeasingInterner::new(names, names, NonZeroU32::new(8).unwrap())
}

fn relations(names: &[&str]) -> Namespace<RelationName> {
    Namespace::new(
        names
            .iter()
            .map(|name| (name.parse().unwrap(), Rewrite::This))
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
async fn declared(names: &MemoryDictionary, store: &MemoryStore) -> Revision {
    let interner = interner(names);
    NamespaceWriteService::new(&interner, store)
        .write(
            &"file".parse().unwrap(),
            NamespaceWrite::Create(relations(&["owner"])),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn a_refused_insert_is_named_as_the_caller_named_it() {
    let (names, store) = (MemoryDictionary::default(), MemoryStore::default());
    declared(&names, &store).await;
    names.intern(Pool::Namespaces, "folder").unwrap();
    let undeclared = FactName {
        subjectset: "folder:root#owner".parse().unwrap(),
        subject: SubjectName::Identity("alice".parse().unwrap()),
    };
    let dictionary = ReadingDictionary(&names);
    let written = FactWriteService::new(&dictionary, &store)
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
    let (names, store) = (MemoryDictionary::default(), MemoryStore::default());
    declared(&names, &store).await;
    let dictionary = ReadingDictionary(&names);
    let clock = Failing(|| ClockFault::Drifted.into());
    let written = FactWriteService::new(&dictionary, &clock)
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
    let written = FactWriteService::new(&dictionary, &store)
        .write(&[], &[owner("alice")])
        .await;
    assert!(
        matches!(written, Err(FactWriteServiceError::Store(_))),
        "{written:?}"
    );
    // A writer that refuses a fact it wasn't asked to add has failed.
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
    let written = FactWriteService::new(&dictionary, &confused)
        .write(&[owner("alice")], &[])
        .await;
    assert!(
        matches!(written, Err(FactWriteServiceError::Store(_))),
        "{written:?}"
    );
}

#[tokio::test]
async fn retiring_a_namespace_never_interned_interns_nothing() {
    let (names, store) = (MemoryDictionary::default(), MemoryStore::default());
    let interner = interner(&names);
    let retired = NamespaceWriteService::new(&interner, &store)
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
                NamespaceWriteError::History(HistoryError::NothingToRetire)
            ))
        ),
        "{retired:?}"
    );
    assert_eq!(names.id(Pool::Namespaces, "file"), None);
}

/// Names are interned before the write, so a conflict leaves them.
#[tokio::test]
async fn a_conflict_leaves_its_names_interned() {
    let (names, store) = (MemoryDictionary::default(), MemoryStore::default());
    let latest = declared(&names, &store).await;
    let interner = interner(&names);
    let written = NamespaceWriteService::new(&interner, &store)
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
    assert!(names.id(Pool::Relations, "viewer").is_some());
}

#[tokio::test]
async fn a_new_version_and_a_retirement_follow_the_latest_version() {
    let (names, store) = (MemoryDictionary::default(), MemoryStore::default());
    let first = declared(&names, &store).await;
    let interner = interner(&names);
    let service = NamespaceWriteService::new(&interner, &store);
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
    let id = sungma::id::NamespaceId(names.id(Pool::Namespaces, "file").unwrap());
    assert!(store.latest(id).is_none());
}
