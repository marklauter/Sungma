//! In-memory simulations of the storage ports.

use std::{
    collections::{BTreeMap, HashMap},
    ops::Range,
    sync::Arc,
};

use parking_lot::{Mutex, MutexGuard};

use sungma::{
    graph::{Fact, Subject, Subjectset},
    id::NamespaceId,
    namespace::Namespace,
    revision::Revision,
    store::{FactReader, NameReader, NamespaceReader, Pool, StoreError},
    version::{History, HistoryError, Version},
};
use sungma_clock::{Clock, ClockFault, DevClock, Reading};
use sungma_write::{
    fact::{check_inserts, opens_span},
    store::{
        FactWrite, FactWriteError, FactWriter, InternError, NameWriter, NamespaceWrite,
        NamespaceWriteError, NamespaceWriter,
    },
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

/// A [`NameWriter`]. Setup code mints through [`MemoryDictionary::intern`],
/// one id at a time from the same counters the leases advance, so its ids
/// never meet a [`sungma_write::intern::LeasingInterner`]'s.
#[derive(Debug, Default)]
pub struct MemoryDictionary {
    pools: Mutex<HashMap<Pool, Names>>,
}

impl MemoryDictionary {
    /// Returns the name's id in `pool`, minting the next one if it's new.
    /// [`InternError::Exhausted`] when the pool has no ids left.
    pub fn intern(&self, pool: Pool, name: &str) -> Result<u32, InternError> {
        let mut pools = self.pools();
        let names = pools.entry(pool).or_default();
        if let Some(&id) = names.ids.get(name) {
            return Ok(id);
        }
        let id = names.lease(1).next().ok_or(InternError::Exhausted(pool))?;
        Ok(names.insert_if_absent(name, id))
    }

    /// The name's id in `pool`, `None` when it was never interned there.
    pub fn id(&self, pool: Pool, name: &str) -> Option<u32> {
        let pools = self.pools();
        pools.get(&pool)?.ids.get(name).copied()
    }

    fn pools(&self) -> MutexGuard<'_, HashMap<Pool, Names>> {
        self.pools.lock()
    }
}

impl NameReader for MemoryDictionary {
    async fn id(&self, pool: Pool, name: &str) -> Result<Option<u32>, StoreError> {
        Ok(MemoryDictionary::id(self, pool, name))
    }

    async fn name(&self, pool: Pool, id: u32) -> Result<Option<String>, StoreError> {
        let pools = self.pools();
        Ok(pools
            .get(&pool)
            .and_then(|names| names.names.get(&id))
            .cloned())
    }
}

impl NameWriter for MemoryDictionary {
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
/// that retired it, if any.
#[derive(Clone, Copy, Debug)]
struct Span {
    written: Revision,
    retired: Option<Revision>,
}

impl Span {
    fn covers(self, revision: Revision) -> bool {
        self.written <= revision && self.retired.is_none_or(|retired| revision < retired)
    }
}

/// Subjects are kept sorted under their subjectset, like sort keys under a
/// partition key, each with the spans it was stored for. A fact retired and
/// stored again has a span for each time.
#[derive(Debug, Default)]
struct Facts {
    spans: HashMap<Subjectset, BTreeMap<Subject, Vec<Span>>>,
}

impl Facts {
    /// Opens a span for `fact` at `revision` when [`opens_span`] says so
    /// under `current`, its namespace's current version, closing any open
    /// span there.
    fn insert(&mut self, fact: &Fact, revision: Revision, current: Option<&Version>) {
        let open = Span {
            written: revision,
            retired: None,
        };
        // The keys are cloned only for a fact not stored before.
        let Some(subjects) = self.spans.get_mut(&fact.subjectset) else {
            let subjects = BTreeMap::from([(fact.subject.clone(), vec![open])]);
            self.spans.insert(fact.subjectset.clone(), subjects);
            return;
        };
        let Some(spans) = subjects.get_mut(&fact.subject) else {
            subjects.insert(fact.subject.clone(), vec![open]);
            return;
        };
        let last = spans.last_mut().filter(|span| span.retired.is_none());
        if !opens_span(fact, last.as_ref().map(|span| span.written), current) {
            return;
        }
        if let Some(span) = last {
            span.retired = Some(revision);
        }
        spans.push(open);
    }

    fn retire(&mut self, fact: &Fact, revision: Revision) {
        let open = self
            .spans
            .get_mut(&fact.subjectset)
            .and_then(|subjects| subjects.get_mut(&fact.subject))
            .and_then(|spans| spans.last_mut())
            .filter(|span| span.retired.is_none());
        if let Some(span) = open {
            span.retired = Some(revision);
        }
    }

    /// The revision that wrote the span covering `revision`.
    fn written(spans: &[Span], revision: Revision) -> Option<Revision> {
        spans
            .iter()
            .find(|span| span.covers(revision))
            .map(|span| span.written)
    }

    fn stored(&self, set: &Subjectset, subject: &Subject, revision: Revision) -> Option<Revision> {
        Self::written(self.spans.get(set)?.get(subject)?, revision)
    }

    fn visible(
        &self,
        set: &Subjectset,
        revision: Revision,
    ) -> impl Iterator<Item = (Subject, Revision)> + '_ {
        self.spans
            .get(set)
            .into_iter()
            .flatten()
            .filter_map(move |(subject, spans)| {
                Self::written(spans, revision).map(|written| (subject.clone(), written))
            })
    }
}

/// Each namespace's history.
#[derive(Debug, Default)]
struct Namespaces(HashMap<NamespaceId, History>);

impl Namespaces {
    /// The version whose span is still open, `None` when the namespace was
    /// never created or is retired.
    fn current(&self, id: NamespaceId) -> Option<&Arc<Version>> {
        self.0.get(&id)?.current()
    }
}

/// Facts and namespace histories on one timeline.
#[derive(Debug, Default)]
struct State {
    facts: Facts,
    namespaces: Namespaces,
    head: u64,
}

impl State {
    fn check_facts(&self, writes: &[FactWrite]) -> Result<(), FactWriteError> {
        check_inserts(writes, |id| self.namespaces.current(id).map(Arc::as_ref))
    }

    fn write_facts(&mut self, writes: &[FactWrite], revision: Revision) {
        let Self {
            facts, namespaces, ..
        } = self;
        for write in writes {
            match write {
                FactWrite::Insert(fact) => {
                    let current = namespaces.current(fact.subjectset.resource.namespace);
                    facts.insert(fact, revision, current.map(Arc::as_ref));
                }
                FactWrite::Retire(fact) => facts.retire(fact, revision),
            }
        }
    }

    /// Refuses a write the namespace isn't at the version of, and a
    /// retirement of a namespace with no current version. Otherwise gives
    /// the version to store, `None` for a retirement.
    fn check_namespace(
        &self,
        id: NamespaceId,
        write: NamespaceWrite,
    ) -> Result<Option<Namespace>, NamespaceWriteError> {
        let current = self
            .namespaces
            .current(id)
            .map(|version| version.revision());
        let (expected, next) = match write {
            NamespaceWrite::Create(namespace) => (None, Some(namespace)),
            NamespaceWrite::NewVersion {
                based_on,
                namespace,
            } => (Some(based_on), Some(namespace)),
            NamespaceWrite::Retire { .. } if current.is_none() => {
                return Err(HistoryError::NothingToRetire.into());
            }
            NamespaceWrite::Retire { based_on } => (Some(based_on), None),
        };
        if current != expected {
            return Err(NamespaceWriteError::Conflict { current });
        }
        Ok(next)
    }

    /// Stores `next` as the namespace's next version, or retires it when
    /// there's none, at `revision`.
    fn store(
        &mut self,
        id: NamespaceId,
        next: Option<Namespace>,
        revision: Revision,
    ) -> Result<(), HistoryError> {
        let history = self.namespaces.0.entry(id).or_default();
        match next {
            Some(namespace) => history.write(namespace, revision),
            None => history.retire(revision),
        }
    }
}

/// Facts and namespace versions, stamped from one [`DevClock`], so both
/// share one timeline. A write takes its reading while it holds the
/// store's lock, so revisions follow the order writes land in.
///
/// [`MemoryStore::declare`] and [`MemoryStore::insert`] seed a store for
/// tests and fixtures without awaiting, since a `DevClock` never needs a
/// wait. `declare` skips the version check `write_namespace` makes;
/// `insert` makes the same checks as `write`.
#[derive(Debug, Default)]
pub struct MemoryStore {
    state: Mutex<State>,
    clock: DevClock,
}

impl MemoryStore {
    /// Stores `namespace` as the namespace's next version, or its first,
    /// whatever version is current, and returns its revision.
    pub fn declare(
        &self,
        id: NamespaceId,
        namespace: Namespace,
    ) -> Result<Revision, NamespaceWriteError> {
        let reading = self.stamp(
            |_| Ok::<_, NamespaceWriteError>(()),
            |state, (), revision| Ok(state.store(id, Some(namespace), revision)?),
        )?;
        Ok(reading.revision)
    }

    /// Stores the facts at one revision and returns it, or refuses them
    /// all as [`FactWriter::write`] does.
    pub fn insert(&self, facts: Vec<Fact>) -> Result<Revision, FactWriteError> {
        let writes: Vec<FactWrite> = facts.into_iter().map(FactWrite::Insert).collect();
        Ok(self.apply(&writes)?.revision)
    }

    /// Checks `writes`, then stores them at one reading of the clock.
    fn apply(&self, writes: &[FactWrite]) -> Result<Reading, FactWriteError> {
        self.stamp(
            |state| state.check_facts(writes),
            |state, (), revision| {
                state.write_facts(writes, revision);
                Ok(())
            },
        )
    }

    /// Under the store's lock: runs `check`, reads the clock, and runs
    /// `store` at the reading's revision, which becomes the head. When
    /// `check` refuses, the clock isn't read.
    fn stamp<T, E: From<ClockFault>>(
        &self,
        check: impl FnOnce(&State) -> Result<T, E>,
        store: impl FnOnce(&mut State, T, Revision) -> Result<(), E>,
    ) -> Result<Reading, E> {
        let mut state = self.state();
        let checked = check(&state)?;
        let reading = self.clock.now()?;
        store(&mut state, checked, reading.revision)?;
        state.head = reading.revision.0;
        Ok(reading)
    }

    /// The current version, `None` when the namespace was never created or
    /// is retired.
    pub fn current(&self, id: NamespaceId) -> Option<Arc<Version>> {
        self.state().namespaces.current(id).cloned()
    }

    /// The current version's namespace.
    pub fn namespace(&self, id: NamespaceId) -> Option<Arc<Namespace>> {
        self.current(id)
            .map(|version| Arc::clone(version.namespace()))
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock()
    }
}

impl FactReader for MemoryStore {
    async fn head(&self) -> Result<Revision, StoreError> {
        Ok(Revision(self.state().head))
    }

    async fn written(
        &self,
        set: &Subjectset,
        subject: &Subject,
        revision: Revision,
    ) -> Result<Option<Revision>, StoreError> {
        Ok(self.state().facts.stored(set, subject, revision))
    }

    async fn subjectsets(
        &self,
        set: &Subjectset,
        revision: Revision,
    ) -> Result<Vec<(Subjectset, Revision)>, StoreError> {
        Ok(self
            .state()
            .facts
            .visible(set, revision)
            .filter_map(|(subject, written)| match subject {
                Subject::Subjectset(nested) => Some((nested, written)),
                _ => None,
            })
            .collect())
    }

    async fn subjects(
        &self,
        set: &Subjectset,
        revision: Revision,
    ) -> Result<Vec<(Subject, Revision)>, StoreError> {
        Ok(self.state().facts.visible(set, revision).collect())
    }
}

impl FactWriter for MemoryStore {
    async fn write(&self, writes: &[FactWrite]) -> Result<Revision, FactWriteError> {
        let reading = self.apply(writes)?;
        self.clock.wait(reading).await?;
        Ok(reading.revision)
    }
}

impl NamespaceReader for MemoryStore {
    async fn version(
        &self,
        namespace: NamespaceId,
        revision: Revision,
    ) -> Result<Option<Arc<Version>>, StoreError> {
        Ok(self
            .state()
            .namespaces
            .0
            .get(&namespace)
            .and_then(|history| history.at(revision))
            .cloned())
    }
}

impl NamespaceWriter for MemoryStore {
    async fn write_namespace(
        &self,
        namespace: NamespaceId,
        write: NamespaceWrite,
    ) -> Result<Revision, NamespaceWriteError> {
        let reading = self.stamp(
            |state| state.check_namespace(namespace, write),
            |state, next, revision| Ok(state.store(namespace, next, revision)?),
        )?;
        self.clock.wait(reading).await?;
        Ok(reading.revision)
    }
}
