//! In-memory simulations of the storage ports.

use std::{
    collections::{BTreeMap, HashMap},
    ops::Range,
    sync::Arc,
};

use parking_lot::{Mutex, MutexGuard};

use sungma::{
    graph::{Fact, Subject, Subjectset},
    id::{NamespaceId, RelationId},
    namespace::Namespace,
    revision::Revision,
    rewrite::Rewrite,
    store::{Dictionary, FactStore, NamespaceStore, Pool, StoreError},
    version::{History, HistoryError, Version},
};
use sungma_write::store::{
    FactWrite, FactWriter, NameStore, NamespaceWrite, NamespaceWriteError, NamespaceWriter,
};

/// One pool's names and ids both ways, and the counter its leases advance.
#[derive(Debug, Default)]
struct Names {
    ids: HashMap<String, u32>,
    names: HashMap<u32, String>,
    next: u32,
}

impl Names {
    fn insert_if_absent(&mut self, name: &str, id: u32) -> u32 {
        if let Some(&stored) = self.ids.get(name) {
            return stored;
        }
        self.ids.insert(name.to_owned(), id);
        self.names.insert(id, name.to_owned());
        id
    }

    fn lease(&mut self, count: u32) -> Range<u32> {
        let start = self.next;
        self.next = start.saturating_add(count);
        start..self.next
    }
}

/// A [`NameStore`]. Setup code mints through [`MemoryDictionary::intern`],
/// one id at a time from the same counters the leases advance, so its ids
/// never meet a [`sungma_write::intern::LeasingInterner`]'s.
#[derive(Debug, Default)]
pub struct MemoryDictionary {
    pools: Mutex<HashMap<Pool, Names>>,
}

impl MemoryDictionary {
    /// Returns the name's id in `pool`, minting the next one if it's new.
    pub fn intern(&self, pool: Pool, name: &str) -> u32 {
        let mut pools = self.pools();
        let names = pools.entry(pool).or_default();
        if let Some(&id) = names.ids.get(name) {
            return id;
        }
        let id = names.lease(1);
        assert!(!id.is_empty(), "{pool:?} exceeds u32 ids");
        names.insert_if_absent(name, id.start)
    }

    fn pools(&self) -> MutexGuard<'_, HashMap<Pool, Names>> {
        self.pools.lock()
    }
}

impl Dictionary for MemoryDictionary {
    async fn lookup(&self, pool: Pool, name: &str) -> Result<Option<u32>, StoreError> {
        let pools = self.pools();
        Ok(pools
            .get(&pool)
            .and_then(|names| names.ids.get(name))
            .copied())
    }

    async fn name(&self, pool: Pool, id: u32) -> Result<Option<String>, StoreError> {
        let pools = self.pools();
        Ok(pools
            .get(&pool)
            .and_then(|names| names.names.get(&id))
            .cloned())
    }
}

impl NameStore for MemoryDictionary {
    async fn insert_if_absent(&self, pool: Pool, name: &str, id: u32) -> Result<u32, StoreError> {
        Ok(self
            .pools()
            .entry(pool)
            .or_default()
            .insert_if_absent(name, id))
    }

    async fn lease(&self, pool: Pool, count: u32) -> Result<Range<u32>, StoreError> {
        Ok(self.pools().entry(pool).or_default().lease(count))
    }
}

/// When a fact was stored: from the revision that wrote it until the one
/// that deleted it, if any.
#[derive(Clone, Copy, Debug)]
struct Span {
    written: Revision,
    deleted: Option<Revision>,
}

impl Span {
    fn covers(self, revision: Revision) -> bool {
        self.written <= revision && self.deleted.is_none_or(|deleted| revision < deleted)
    }
}

/// Subjects are kept sorted under their subjectset, like sort keys under a
/// partition key, each with the spans it was stored for. A fact deleted and
/// stored again has a span for each time.
#[derive(Debug, Default)]
struct Facts {
    spans: HashMap<Subjectset, BTreeMap<Subject, Vec<Span>>>,
    head: u64,
}

impl Facts {
    fn write(&mut self, writes: &[FactWrite]) -> Revision {
        self.head += 1;
        let revision = Revision(self.head);
        for write in writes {
            match write {
                FactWrite::Insert(fact) => {
                    let spans = self
                        .spans
                        .entry(fact.subjectset.clone())
                        .or_default()
                        .entry(fact.subject.clone())
                        .or_default();
                    if spans.last().is_none_or(|span| span.deleted.is_some()) {
                        spans.push(Span {
                            written: revision,
                            deleted: None,
                        });
                    }
                }
                FactWrite::Delete(fact) => {
                    let open = self
                        .spans
                        .get_mut(&fact.subjectset)
                        .and_then(|subjects| subjects.get_mut(&fact.subject))
                        .and_then(|spans| spans.last_mut())
                        .filter(|span| span.deleted.is_none());
                    if let Some(span) = open {
                        span.deleted = Some(revision);
                    }
                }
            }
        }
        revision
    }

    fn contains(&self, set: &Subjectset, subject: &Subject, revision: Revision) -> bool {
        self.spans
            .get(set)
            .and_then(|subjects| subjects.get(subject))
            .is_some_and(|spans| spans.iter().any(|span| span.covers(revision)))
    }

    fn visible(&self, set: &Subjectset, revision: Revision) -> impl Iterator<Item = Subject> + '_ {
        self.spans
            .get(set)
            .into_iter()
            .flatten()
            .filter(move |(_, spans)| spans.iter().any(|span| span.covers(revision)))
            .map(|(subject, _)| subject.clone())
    }
}

/// Each write produces the next revision.
#[derive(Debug, Default)]
pub struct MemoryFactStore {
    facts: Mutex<Facts>,
}

impl MemoryFactStore {
    /// Stores the fact at the next revision and returns it.
    pub fn insert(&self, fact: Fact) -> Revision {
        self.facts().write(&[FactWrite::Insert(fact)])
    }

    fn facts(&self) -> MutexGuard<'_, Facts> {
        self.facts.lock()
    }
}

impl FactStore for MemoryFactStore {
    async fn head(&self) -> Result<Revision, StoreError> {
        Ok(Revision(self.facts().head))
    }

    async fn contains(
        &self,
        set: &Subjectset,
        subject: &Subject,
        revision: Revision,
    ) -> Result<bool, StoreError> {
        Ok(self.facts().contains(set, subject, revision))
    }

    async fn subjectsets(
        &self,
        set: &Subjectset,
        revision: Revision,
    ) -> Result<Vec<Subjectset>, StoreError> {
        Ok(self
            .facts()
            .visible(set, revision)
            .filter_map(|subject| match subject {
                Subject::Subjectset(nested) => Some(nested),
                _ => None,
            })
            .collect())
    }

    async fn subjects(
        &self,
        set: &Subjectset,
        revision: Revision,
    ) -> Result<Vec<Subject>, StoreError> {
        Ok(self.facts().visible(set, revision).collect())
    }
}

impl FactWriter for MemoryFactStore {
    async fn write(&self, writes: &[FactWrite]) -> Result<Revision, StoreError> {
        Ok(self.facts().write(writes))
    }
}

/// Each namespace's history.
#[derive(Debug, Default)]
struct Versions {
    namespaces: HashMap<NamespaceId, History>,
    head: u64,
}

impl Versions {
    /// The version whose span is still open, `None` when the namespace was
    /// never created or is retired.
    fn current(&self, id: NamespaceId) -> Option<&Arc<Version>> {
        self.namespaces.get(&id)?.current()
    }

    /// Refuses a write the namespace isn't at the version of, then stores it.
    fn write(
        &mut self,
        id: NamespaceId,
        write: NamespaceWrite,
    ) -> Result<Revision, NamespaceWriteError> {
        let current = self.current(id).map(|version| version.revision());
        let (expected, next) = match write {
            NamespaceWrite::Create(namespace) => (None, Some(namespace)),
            NamespaceWrite::NewVersion {
                based_on,
                namespace,
            } => (Some(based_on), Some(namespace)),
            NamespaceWrite::Retire { based_on } => (Some(based_on), None),
        };
        if current != expected {
            return Err(NamespaceWriteError::Conflict { current });
        }
        Ok(self.store(id, next)?)
    }

    /// Stores `next` as the namespace's next version, or retires it when
    /// there's none, at the next revision.
    fn store(
        &mut self,
        id: NamespaceId,
        next: Option<Namespace>,
    ) -> Result<Revision, HistoryError> {
        let revision = Revision(self.head + 1);
        let history = self.namespaces.entry(id).or_default();
        match next {
            Some(namespace) => history.write(namespace, revision),
            None => history.retire(revision),
        }?;
        self.head += 1;
        Ok(revision)
    }
}

/// Every namespace's versions. Reads see each namespace's current version;
/// reading at a revision comes with the readers.
///
/// For the reader: revisions here come from the store's own `head`, not
/// from the fact store's, so epochs and fact revisions aren't on one
/// timeline yet. A relation added back at namespace revision 4 sits
/// beside facts at fact revision 500, so every old fact would pass its
/// epoch. Facts and namespaces need one revision source before reads
/// apply epochs.
#[derive(Debug, Default)]
pub struct MemoryNamespaceStore {
    versions: Mutex<Versions>,
}

impl MemoryNamespaceStore {
    /// Stores `namespace` as the namespace's next version, or its first,
    /// whatever version is current, and returns its revision.
    pub fn declare(&self, id: NamespaceId, namespace: Namespace) -> Result<Revision, HistoryError> {
        self.versions().store(id, Some(namespace))
    }

    /// The current version, `None` when the namespace was never created or
    /// is retired.
    pub fn version(&self, id: NamespaceId) -> Option<Arc<Version>> {
        self.versions().current(id).cloned()
    }

    /// The current version's namespace.
    pub fn namespace(&self, id: NamespaceId) -> Option<Arc<Namespace>> {
        self.version(id)
            .map(|version| Arc::clone(version.namespace()))
    }

    fn versions(&self) -> MutexGuard<'_, Versions> {
        self.versions.lock()
    }
}

/// For the reader: `rewrite` reads the current version, so it neither
/// applies epochs nor reads at a revision.
///
/// - A relation dropped and then declared again returns its rewrite, and
///   facts written before the new epoch count again. In the sample in
///   `docs/specs/one-timeline.md`, Bob's ban comes back at revision 35.
/// - Older versions and closed spans are stored but never read. After a
///   retirement `rewrite` returns `None`, so replaying a decision made
///   before it denies what was granted. Spec tests 5, 7 and 8 need a read
///   at the version whose span covers the revision.
impl NamespaceStore for MemoryNamespaceStore {
    async fn rewrite(
        &self,
        namespace: NamespaceId,
        relation: RelationId,
    ) -> Result<Option<Arc<Rewrite>>, StoreError> {
        Ok(self
            .versions()
            .current(namespace)
            .and_then(|version| version.namespace().rewrite(&relation))
            .cloned())
    }
}

impl NamespaceWriter for MemoryNamespaceStore {
    async fn write_namespace(
        &self,
        namespace: NamespaceId,
        write: NamespaceWrite,
    ) -> Result<Revision, NamespaceWriteError> {
        self.versions().write(namespace, write)
    }
}
