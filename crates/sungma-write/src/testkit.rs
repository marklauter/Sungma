//! The tests every store runs, against its own ports. A store's crate
//! implements [`Stores`] and calls [`store_tests!`](crate::store_tests),
//! which writes a test for each function here. Facts and namespace
//! versions are stamped from a counting clock that starts at 1, as
//! `DevClock` counts.

use std::{collections::HashSet, future::Future, sync::Arc};

use proptest::{prop_assert, prop_assert_eq, test_runner::TestCaseError};
use sungma::{
    closure::{Closure, Expansion},
    graph::{Fact, Resource, Subject, Subjectset},
    id::{NamespaceId, RelationId},
    name::{Identity, ResourceId},
    namespace::Namespace,
    revision::Revision,
    rewrite::Rewrite::{self, FactTo, This},
    store::{FactReader, NamespaceReader, Pool},
    version::{HistoryError, Version},
};

use crate::store::{
    FactWrite::{self, Insert, Retire},
    FactWriteError, FactWriter, NameWriter,
    NamespaceWrite::{self, Create, NewVersion},
    NamespaceWriteError, NamespaceWriter,
};

/// One store's ports, empty when the store is made.
pub trait Stores: Send + Sync + 'static {
    type Reader: FactReader + NamespaceReader + Send + Sync;
    type Writer: FactWriter + NamespaceWriter + Send + Sync;
    type Names: NameWriter + Send + Sync;

    fn reader(&self) -> &Self::Reader;
    fn writer(&self) -> &Self::Writer;
    fn names(&self) -> &Self::Names;
}

/// Writes a test for each function in [`crate::testkit`], against the
/// stores `$make()` gives, an `async fn` that makes empty ones. The
/// history property runs `$cases` cases.
#[macro_export]
macro_rules! store_tests {
    ($make:path, cases = $cases:expr) => {
        $crate::store_tests!(@each $make;
            each_write_is_the_next_revision,
            a_batch_lands_at_one_revision,
            a_retired_fact_stays_visible_before_its_retirement,
            a_fact_stored_again_is_visible_in_each_span,
            inserting_a_stored_fact_keeps_its_revision,
            retiring_an_absent_fact_changes_nothing,
            a_batch_that_writes_a_fact_twice_lands_its_net_change,
            subjects_are_read_in_order,
            an_insert_under_an_undeclared_relation_stores_nothing,
            inserting_a_fact_from_before_its_epoch_opens_a_new_span,
            creating_a_namespace_that_has_a_current_version_is_refused,
            a_new_version_closes_the_current_one,
            a_write_based_on_an_older_version_stores_nothing,
            a_retired_namespace_declares_nothing_until_it_is_created_again,
            a_read_between_two_versions_sees_the_earlier_one,
            the_one_timeline_sample,
            a_path_through_a_fact_from_before_its_epoch_is_closed,
        );

        /// Writers on separate threads, all based on one version, race
        /// for it.
        #[::tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn of_writes_based_on_one_version_exactly_one_succeeds() {
            $crate::testkit::of_writes_based_on_one_version_exactly_one_succeeds(|| $make())
                .await;
        }

        ::proptest::proptest! {
            #![proptest_config(::proptest::test_runner::Config::with_cases($cases))]

            /// A read at a revision sees exactly the facts that applying
            /// every batch up to it, in order, leaves stored.
            #[test]
            fn a_read_at_a_revision_sees_the_facts_stored_then(
                batches in $crate::testkit::batches(),
            ) {
                let runtime = ::tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                runtime.block_on(async {
                    $crate::testkit::reads_match_the_history($make().await, batches).await
                })?;
            }
        }
    };
    (@each $make:path; $($test:ident,)*) => {
        $(
            #[::tokio::test]
            async fn $test() {
                $crate::testkit::$test($make().await).await;
            }
        )*
    };
}

// Names stand in for ids, so a store that keeps names can print documents.

/// Interns a name for the namespace and for every relation it names.
async fn intern<S: Stores>(stores: &S, id: NamespaceId, namespace: &Namespace) {
    let names = stores.names();
    names
        .insert_if_absent(Pool::Namespaces, &format!("ns{}", id.0), id.0)
        .await
        .unwrap();
    let mut relations = Vec::new();
    namespace
        .map(|relation| {
            relations.push(*relation);
            *relation
        })
        .unwrap();
    for relation in relations {
        names
            .insert_if_absent(Pool::Relations, &format!("r{}", relation.0), relation.0)
            .await
            .unwrap();
    }
}

/// Writes to the namespace, interning its names first.
async fn write_namespace<S: Stores>(
    stores: &S,
    id: NamespaceId,
    write: NamespaceWrite,
) -> Result<Revision, NamespaceWriteError> {
    if let Create(namespace) | NewVersion { namespace, .. } = &write {
        intern(stores, id, namespace).await;
    }
    stores.writer().write_namespace(id, write).await
}

async fn create<S: Stores>(stores: &S, id: NamespaceId, namespace: Namespace) -> Revision {
    write_namespace(stores, id, Create(namespace))
        .await
        .unwrap()
}

fn declaring(relations: &[RelationId]) -> Namespace {
    Namespace::new(relations.iter().map(|&relation| (relation, This)).collect()).unwrap()
}

fn person(name: &str) -> Subject {
    Subject::Identity(Identity::new_unchecked(name))
}

fn resource(namespace: NamespaceId, id: &str) -> Resource {
    Resource {
        namespace,
        id: ResourceId::new_unchecked(id),
    }
}

fn at(resource: Resource, relation: RelationId) -> Subjectset {
    Subjectset { resource, relation }
}

// Facts through the write port: facts keep their history, so a read at
// any revision sees the facts stored then.

const DOC: NamespaceId = NamespaceId(0);
const OWNER: RelationId = RelationId(2);
const MEMBER: RelationId = RelationId(5);

/// The namespace declares both relations, at revision 1.
async fn declared<S: Stores>(stores: S) -> S {
    create(&stores, DOC, declaring(&[OWNER, MEMBER])).await;
    stores
}

fn set() -> Subjectset {
    at(resource(DOC, "doc"), OWNER)
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

async fn stored<S: Stores>(stores: &S, revision: u64) -> Vec<Subject> {
    stores
        .reader()
        .subjects(&set(), Revision(revision))
        .await
        .unwrap()
        .into_iter()
        .map(|(subject, _)| subject)
        .collect()
}

async fn written<S: Stores>(stores: &S, revision: u64) -> Option<Revision> {
    stores
        .reader()
        .written(&set(), &alice(), Revision(revision))
        .await
        .unwrap()
}

async fn write<S: Stores>(stores: &S, writes: &[FactWrite]) -> Revision {
    stores.writer().write(writes).await.unwrap()
}

pub async fn each_write_is_the_next_revision<S: Stores>(stores: S) {
    let stores = declared(stores).await;
    assert_eq!(write(&stores, &[Insert(fact(alice()))]).await, Revision(2));
    assert_eq!(write(&stores, &[]).await, Revision(3));
    assert_eq!(stores.reader().head().await.unwrap(), Revision(3));
}

pub async fn a_batch_lands_at_one_revision<S: Stores>(stores: S) {
    let stores = declared(stores).await;
    let revision = write(&stores, &[Insert(fact(alice())), Insert(fact(bob()))]).await;
    assert_eq!(revision, Revision(2));
    assert_eq!(stored(&stores, 1).await, []);
    assert_eq!(stored(&stores, 2).await, [alice(), bob()]);
}

pub async fn a_retired_fact_stays_visible_before_its_retirement<S: Stores>(stores: S) {
    let stores = declared(stores).await;
    write(&stores, &[Insert(fact(alice()))]).await;
    write(&stores, &[Retire(fact(alice()))]).await;
    assert_eq!(written(&stores, 2).await, Some(Revision(2)));
    assert_eq!(written(&stores, 3).await, None);
}

pub async fn a_fact_stored_again_is_visible_in_each_span<S: Stores>(stores: S) {
    let stores = declared(stores).await;
    write(&stores, &[Insert(fact(alice()))]).await;
    write(&stores, &[Retire(fact(alice()))]).await;
    write(&stores, &[Insert(fact(alice()))]).await;
    let mut seen = Vec::new();
    for revision in 2..=4 {
        seen.push(written(&stores, revision).await);
    }
    assert_eq!(seen, [Some(Revision(2)), None, Some(Revision(4))]);
}

pub async fn inserting_a_stored_fact_keeps_its_revision<S: Stores>(stores: S) {
    let stores = declared(stores).await;
    write(&stores, &[Insert(fact(alice()))]).await;
    write(&stores, &[Insert(fact(alice()))]).await;
    assert_eq!(written(&stores, 3).await, Some(Revision(2)));
    write(&stores, &[Retire(fact(alice()))]).await;
    assert_eq!(written(&stores, 4).await, None);
}

pub async fn retiring_an_absent_fact_changes_nothing<S: Stores>(stores: S) {
    let stores = declared(stores).await;
    write(&stores, &[Retire(fact(alice()))]).await;
    write(&stores, &[Insert(fact(alice()))]).await;
    write(&stores, &[Retire(fact(bob()))]).await;
    assert_eq!(stored(&stores, 4).await, [alice()]);
}

/// A batch may write one fact more than once; it lands what applying its
/// writes in order leaves.
pub async fn a_batch_that_writes_a_fact_twice_lands_its_net_change<S: Stores>(stores: S) {
    let stores = declared(stores).await;
    write(&stores, &[Insert(fact(alice())), Retire(fact(alice()))]).await;
    assert_eq!(written(&stores, 2).await, None);
    write(&stores, &[Insert(fact(alice())), Insert(fact(alice()))]).await;
    assert_eq!(written(&stores, 3).await, Some(Revision(3)));
    let reopened = write(&stores, &[Retire(fact(alice())), Insert(fact(alice()))]).await;
    assert_eq!(written(&stores, 4).await, Some(reopened));
    write(&stores, &[Insert(fact(alice())), Retire(fact(alice()))]).await;
    assert_eq!(written(&stores, 5).await, None);
    assert_eq!(written(&stores, 4).await, Some(reopened));
}

/// Subjects come back as they sort: identities, then subjectsets, then
/// resource members.
pub async fn subjects_are_read_in_order<S: Stores>(stores: S) {
    let stores = declared(stores).await;
    let member = Subject::Subjectset(Subjectset {
        relation: MEMBER,
        ..set()
    });
    let resource = Subject::ResourceMember(set().resource);
    let mut subjects = vec![resource, member, bob(), alice(), person("al")];
    let writes: Vec<_> = subjects.iter().cloned().map(fact).map(Insert).collect();
    write(&stores, &writes).await;
    subjects.sort();
    assert_eq!(stored(&stores, 2).await, subjects);
}

/// An insert the current versions don't declare is refused with the whole
/// batch, and takes no revision.
pub async fn an_insert_under_an_undeclared_relation_stores_nothing<S: Stores>(stores: S) {
    let stores = declared(stores).await;
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
            subjectset: at(resource(NamespaceId(7), "doc"), OWNER),
            subject: alice(),
        },
    ];
    for refused in refusals {
        let result = stores
            .writer()
            .write(&[Insert(fact(bob())), Insert(refused.clone())])
            .await;
        assert!(
            matches!(&result, Err(FactWriteError::Undeclared(fact)) if *fact == refused),
            "{result:?}"
        );
    }
    assert_eq!(stores.reader().head().await.unwrap(), Revision(1));
    assert_eq!(stored(&stores, 1).await, []);
    let member = Subject::Subjectset(Subjectset {
        relation: MEMBER,
        ..set()
    });
    let resource = Subject::ResourceMember(set().resource);
    write(
        &stores,
        &[
            Insert(fact(member)),
            Insert(fact(resource)),
            Retire(fact(bob())),
        ],
    )
    .await;
}

/// The current version at the head.
async fn current<S: Stores>(stores: &S, id: NamespaceId) -> Option<Arc<Version>> {
    let reader = stores.reader();
    let head = reader.head().await.unwrap();
    reader.version(id, head).await.unwrap()
}

/// Writes `namespace` as the next version of whatever is current.
async fn redeclare<S: Stores>(stores: &S, id: NamespaceId, namespace: Namespace) -> Revision {
    let based_on = current(stores, id).await.unwrap().revision();
    write_namespace(
        stores,
        id,
        NewVersion {
            based_on,
            namespace,
        },
    )
    .await
    .unwrap()
}

/// A fact written before its relation's epoch is stored again in a new
/// span when it's inserted again, and one written after keeps its span.
pub async fn inserting_a_fact_from_before_its_epoch_opens_a_new_span<S: Stores>(stores: S) {
    let stores = declared(stores).await;
    write(&stores, &[Insert(fact(alice()))]).await;
    redeclare(&stores, DOC, declaring(&[MEMBER])).await;
    let back = redeclare(&stores, DOC, declaring(&[OWNER, MEMBER])).await;
    assert_eq!(back, Revision(4));
    write(&stores, &[Insert(fact(alice()))]).await;
    assert_eq!(written(&stores, 4).await, Some(Revision(2)));
    assert_eq!(written(&stores, 5).await, Some(Revision(5)));
    write(&stores, &[Insert(fact(alice()))]).await;
    assert_eq!(written(&stores, 6).await, Some(Revision(5)));
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

/// `(insert, set, subject)`: an insert or a retirement of the fact of
/// `sets()[set]` and `subjects()[subject]`.
pub type Write = (bool, usize, usize);

/// Up to 10 batches of up to 4 writes each.
pub fn batches() -> impl proptest::strategy::Strategy<Value = Vec<Vec<Write>>> {
    use proptest::{collection::vec, prelude::any};
    vec(vec((any::<bool>(), 0..2usize, 0..3usize), 0..4), 0..10)
}

/// Writes `batches` and compares every read at every revision with a model
/// that applies each batch's writes in order to a set of facts.
pub async fn reads_match_the_history<S: Stores>(
    stores: S,
    batches: Vec<Vec<Write>>,
) -> Result<(), TestCaseError> {
    let stores = declared(stores).await;
    let reader = stores.reader();
    let first = reader.head().await.unwrap().0;
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
                if insert { Insert(fact) } else { Retire(fact) }
            })
            .collect();
        let revision = stores.writer().write(&writes).await.unwrap();
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
    prop_assert_eq!(reader.head().await.unwrap(), head);
    for (i, state) in states.iter().enumerate() {
        let revision = Revision(first + i as u64);
        for (i, set) in sets.iter().enumerate() {
            let expected: HashSet<Subject> = state
                .iter()
                .filter(|&&(stored, _)| stored == i)
                .map(|&(_, subject)| subjects[subject].clone())
                .collect();
            let stored = reader.subjects(set, revision).await.unwrap();
            prop_assert_eq!(stored.len(), expected.len(), "{:?}", revision);
            prop_assert!(stored.iter().all(|(_, written)| *written <= revision));
            let stored: HashSet<Subject> = stored.into_iter().map(|(subject, _)| subject).collect();
            prop_assert_eq!(stored, expected.clone());
            let nested: HashSet<Subject> = reader
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
                let written = reader.written(set, subject, revision).await.unwrap();
                prop_assert_eq!(written.is_some(), expected.contains(subject));
            }
        }
    }
    Ok(())
}

// Namespaces through the write port: a write is refused unless the
// namespace is still at the version it was based on, and a read at a
// revision sees the version whose span covers it.

const FILE: NamespaceId = NamespaceId(0);
const BANNED: RelationId = RelationId(1);
const FILE_OWNER: RelationId = RelationId(0);

fn conflict(result: Result<Revision, NamespaceWriteError>) -> Option<Revision> {
    match result {
        Err(NamespaceWriteError::Conflict { current }) => current,
        other => panic!("expected a conflict, got {other:?}"),
    }
}

/// The relation's rewrite at the latest revision.
async fn rewrite<S: Stores>(stores: &S, relation: RelationId) -> Option<Arc<Rewrite>> {
    let version = current(stores, FILE).await?;
    version.namespace().rewrite(&relation).cloned()
}

fn refused_retirement(result: Result<Revision, NamespaceWriteError>) -> bool {
    matches!(
        result,
        Err(NamespaceWriteError::History(HistoryError::NothingToRetire))
    )
}

pub async fn creating_a_namespace_that_has_a_current_version_is_refused<S: Stores>(stores: S) {
    let created = create(&stores, FILE, declaring(&[FILE_OWNER])).await;
    assert_eq!(created, Revision(1));
    let refused = write_namespace(&stores, FILE, Create(declaring(&[BANNED]))).await;
    assert_eq!(conflict(refused), Some(created));
    assert!(rewrite(&stores, BANNED).await.is_none());
}

pub async fn a_new_version_closes_the_current_one<S: Stores>(stores: S) {
    let first = create(&stores, FILE, declaring(&[FILE_OWNER, BANNED])).await;
    let second = redeclare(&stores, FILE, declaring(&[FILE_OWNER])).await;
    assert_eq!(second, Revision(2));
    let current = current(&stores, FILE).await.unwrap();
    assert_eq!(current.revision(), second);
    assert_eq!(current.epoch(FILE_OWNER), Some(first));
    assert!(rewrite(&stores, FILE_OWNER).await.is_some());
    assert!(rewrite(&stores, BANNED).await.is_none());
}

pub async fn a_write_based_on_an_older_version_stores_nothing<S: Stores>(stores: S) {
    let first = create(&stores, FILE, declaring(&[FILE_OWNER])).await;
    let based_on = |namespace| NewVersion {
        based_on: first,
        namespace,
    };
    let second = write_namespace(&stores, FILE, based_on(declaring(&[FILE_OWNER])))
        .await
        .unwrap();
    let refused = write_namespace(&stores, FILE, based_on(declaring(&[BANNED]))).await;
    assert_eq!(conflict(refused), Some(second));
    let refused = write_namespace(&stores, FILE, NamespaceWrite::Retire { based_on: first }).await;
    assert_eq!(conflict(refused), Some(second));
    assert_eq!(current(&stores, FILE).await.unwrap().revision(), second);
    assert!(rewrite(&stores, BANNED).await.is_none());
}

/// Writers on separate tasks, all based on one version, race for it. Each
/// round gets stores from `make`.
pub async fn of_writes_based_on_one_version_exactly_one_succeeds<S, F, M>(make: M)
where
    S: Stores,
    F: Future<Output = S>,
    M: Fn() -> F,
{
    for _ in 0..10 {
        let stores = Arc::new(make().await);
        let first = create(&*stores, FILE, declaring(&[FILE_OWNER])).await;
        intern(&*stores, FILE, &declaring(&[BANNED])).await;
        let writers: Vec<_> = [FILE_OWNER, BANNED, FILE_OWNER, BANNED]
            .into_iter()
            .map(|relation| {
                let stores = Arc::clone(&stores);
                tokio::spawn(async move {
                    let write = NewVersion {
                        based_on: first,
                        namespace: declaring(&[relation]),
                    };
                    stores.writer().write_namespace(FILE, write).await.is_ok()
                })
            })
            .collect();
        let mut stored = 0;
        for writer in writers {
            stored += usize::from(writer.await.unwrap());
        }
        assert_eq!(stored, 1);
    }
}

pub async fn a_retired_namespace_declares_nothing_until_it_is_created_again<S: Stores>(stores: S) {
    let first = create(&stores, FILE, declaring(&[FILE_OWNER])).await;
    let retire = || NamespaceWrite::Retire { based_on: first };
    let retired = write_namespace(&stores, FILE, retire()).await.unwrap();
    assert!(current(&stores, FILE).await.is_none());
    assert!(rewrite(&stores, FILE_OWNER).await.is_none());
    let refused = write_namespace(
        &stores,
        FILE,
        NewVersion {
            based_on: first,
            namespace: declaring(&[FILE_OWNER]),
        },
    )
    .await;
    assert_eq!(conflict(refused), None);
    assert!(refused_retirement(
        write_namespace(&stores, FILE, retire()).await
    ));
    assert!(refused_retirement(
        write_namespace(&stores, NamespaceId(3), retire()).await
    ));
    let again = create(&stores, FILE, declaring(&[FILE_OWNER])).await;
    assert!(again > retired);
    assert_eq!(
        current(&stores, FILE).await.unwrap().epoch(FILE_OWNER),
        Some(again)
    );
}

pub async fn a_read_between_two_versions_sees_the_earlier_one<S: Stores>(stores: S) {
    let reader = stores.reader();
    let opened = |version: Option<Arc<Version>>| version.map(|version| version.revision());
    assert_eq!(
        opened(reader.version(FILE, Revision(0)).await.unwrap()),
        None
    );
    let first = create(&stores, FILE, declaring(&[FILE_OWNER])).await;
    create(&stores, NamespaceId(1), declaring(&[])).await;
    let second = write_namespace(
        &stores,
        FILE,
        NewVersion {
            based_on: first,
            namespace: declaring(&[FILE_OWNER, BANNED]),
        },
    )
    .await
    .unwrap();
    let retired = write_namespace(&stores, FILE, NamespaceWrite::Retire { based_on: second })
        .await
        .unwrap();
    assert_eq!(
        [first, second, retired].map(|revision| revision.0),
        [1, 3, 4]
    );
    let mut seen = Vec::new();
    for revision in 0..=4 {
        seen.push(opened(
            reader.version(FILE, Revision(revision)).await.unwrap(),
        ));
    }
    assert_eq!(seen, [None, Some(first), Some(first), Some(second), None]);
    assert!(
        reader
            .version(NamespaceId(2), Revision(4))
            .await
            .unwrap()
            .is_none()
    );
}

// The sample in `docs/specs/one-timeline.md`, judged by the closure: facts
// and namespace versions on one timeline, read at each revision.

fn file_set(relation: RelationId) -> Subjectset {
    at(resource(FILE, "a"), relation)
}

async fn grant<S: Stores>(stores: &S, relation: RelationId, who: &str) -> Revision {
    let fact = Fact {
        subjectset: file_set(relation),
        subject: person(who),
    };
    write(stores, &[Insert(fact)]).await
}

/// Whether each of `owner@carol`, `banned@bob` and `banned@erin` counts.
async fn counted<S: Stores>(stores: &S, revision: Revision) -> [bool; 3] {
    let reader = stores.reader();
    let mut counts = [false; 3];
    for (count, (relation, who)) in
        counts
            .iter_mut()
            .zip([(FILE_OWNER, "carol"), (BANNED, "bob"), (BANNED, "erin")])
    {
        let set = file_set(relation);
        let closure = Closure::new(reader, reader, &set, revision);
        *count = closure.contains(&person(who)).await.unwrap();
    }
    counts
}

/// The sample's revisions are 10, 12, 14 and so on; the clock counts 1, 2,
/// 3, in the same order.
pub async fn the_one_timeline_sample<S: Stores>(stores: S) {
    let version_1 = create(&stores, FILE, declaring(&[FILE_OWNER, BANNED])).await;
    grant(&stores, FILE_OWNER, "carol").await;
    let bob = grant(&stores, BANNED, "bob").await;
    let version_2 = write_namespace(
        &stores,
        FILE,
        NewVersion {
            based_on: version_1,
            namespace: declaring(&[FILE_OWNER]),
        },
    )
    .await
    .unwrap();
    let version_3 = write_namespace(
        &stores,
        FILE,
        NewVersion {
            based_on: version_2,
            namespace: declaring(&[FILE_OWNER, BANNED]),
        },
    )
    .await
    .unwrap();
    let erin = grant(&stores, BANNED, "erin").await;
    let retired = write_namespace(
        &stores,
        FILE,
        NamespaceWrite::Retire {
            based_on: version_3,
        },
    )
    .await
    .unwrap();
    let version_4 = create(&stores, FILE, declaring(&[FILE_OWNER, BANNED])).await;
    let bob_again = grant(&stores, BANNED, "bob").await;

    // Read at 15: version 1, and both facts were written after epoch 10.
    assert_eq!(counted(&stores, bob).await, [true, true, false]);
    // Read at 25: version 2 doesn't declare `banned`.
    assert_eq!(counted(&stores, version_2).await, [true, false, false]);
    // Read at 35: `banned` has epoch 30, after Bob's fact but not Erin's.
    assert_eq!(counted(&stores, erin).await, [true, false, true]);
    // Read at 45: retired, so no version declares anything.
    assert_eq!(counted(&stores, retired).await, [false; 3]);
    // Read at 55: every epoch is 50, and every fact is older.
    assert_eq!(counted(&stores, version_4).await, [false; 3]);
    // Read at 65: Bob's fact, written again, opened a new span after 50.
    assert_eq!(counted(&stores, bob_again).await, [false, true, false]);
}

const GROUP: NamespaceId = NamespaceId(1);
const VIEWER: RelationId = RelationId(2);
const PARENT: RelationId = RelationId(3);
const INHERITED: RelationId = RelationId(4);

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
async fn reached<S: Stores>(stores: &S, revision: Revision) -> (bool, bool, Expansion) {
    let reader = stores.reader();
    let viewer = file_set(VIEWER);
    let inherited = file_set(INHERITED);
    let alice = Closure::new(reader, reader, &viewer, revision)
        .contains(&person("alice"))
        .await
        .unwrap();
    let bob = Closure::new(reader, reader, &inherited, revision)
        .contains(&person("bob"))
        .await
        .unwrap();
    let expanded = Closure::new(reader, reader, &viewer, revision)
        .expand()
        .await
        .unwrap()
        .unwrap();
    (alice, bob, expanded)
}

/// A nested subjectset and a parent link written before their relation's
/// epoch don't count, though the facts they lead to do.
pub async fn a_path_through_a_fact_from_before_its_epoch_is_closed<S: Stores>(stores: S) {
    create(&stores, GROUP, declaring(&[MEMBER])).await;
    let first = create(&stores, FILE, file(&[VIEWER, PARENT, INHERITED])).await;
    let eng = Subject::Subjectset(at(resource(GROUP, "eng"), MEMBER));
    let nested = Fact {
        subjectset: file_set(VIEWER),
        subject: eng.clone(),
    };
    let parent = Fact {
        subjectset: file_set(PARENT),
        subject: Subject::ResourceMember(resource(FILE, "root")),
    };
    let root = || Fact {
        subjectset: at(resource(FILE, "root"), VIEWER),
        subject: person("bob"),
    };
    let grants = [
        Insert(Fact {
            subjectset: at(resource(GROUP, "eng"), MEMBER),
            subject: person("alice"),
        }),
        Insert(nested.clone()),
        Insert(parent.clone()),
        Insert(root()),
    ];
    let granted = write(&stores, &grants).await;
    let (alice, bob, expanded) = reached(&stores, granted).await;
    assert!(alice && bob);
    assert_eq!(expanded, Expansion::Subjects(vec![eng.clone()]));

    // `viewer` and `parent` leave and come back, so their epochs move past
    // every fact above. Bob's viewer fact on the root is written again, so
    // only the parent link is from before the epoch.
    let dropped = write_namespace(
        &stores,
        FILE,
        NewVersion {
            based_on: first,
            namespace: declaring(&[FILE_OWNER]),
        },
    )
    .await
    .unwrap();
    write_namespace(
        &stores,
        FILE,
        NewVersion {
            based_on: dropped,
            namespace: file(&[VIEWER, PARENT, INHERITED]),
        },
    )
    .await
    .unwrap();
    let rewritten = write(&stores, &[Insert(root())]).await;
    let (alice, bob, expanded) = reached(&stores, rewritten).await;
    assert!(!alice && !bob);
    assert_eq!(expanded, Expansion::Subjects(vec![]));

    let again = write(&stores, &[Insert(nested), Insert(parent)]).await;
    let (alice, bob, _) = reached(&stores, again).await;
    assert!(alice && bob);
}
