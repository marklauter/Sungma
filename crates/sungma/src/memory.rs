//! In-memory simulations of the storage ports.

use std::{
    collections::{BTreeMap, HashMap},
    ops::Range,
    sync::{Arc, Mutex, MutexGuard},
};

use crate::{
    clock::Revision,
    graph::{Fact, Subject, Subjectset},
    id::{RelationId, TheoryId},
    rewrite::Rewrite,
    store::{
        Dictionary, FactStore, FactWrite, FactWriter, NameStore, Pool, StoreError, TheoryStore,
    },
    theory::Theory,
};

/// The ports take `&self` so a store can be shared across concurrent
/// requests, so its contents sit behind a mutex. Setup code calls the
/// synchronous inherent methods, which share the ports' code.
fn lock<'a, T>(mutex: &'a Mutex<T>, what: &str) -> Result<MutexGuard<'a, T>, StoreError> {
    mutex
        .lock()
        .map_err(|_| StoreError(format!("{what} poisoned")))
}

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
/// never meet a [`crate::intern::LeasingInterner`]'s.
#[derive(Debug, Default)]
pub struct MemoryDictionary {
    pools: Mutex<HashMap<Pool, Names>>,
}

impl MemoryDictionary {
    /// Returns the name's id in `pool`, minting the next one if it's new.
    pub fn intern(&self, pool: Pool, name: &str) -> u32 {
        let mut pools = self.pools.lock().expect("dictionary poisoned");
        let names = pools.entry(pool).or_default();
        if let Some(&id) = names.ids.get(name) {
            return id;
        }
        let id = names.lease(1);
        assert!(!id.is_empty(), "{pool:?} exceeds u32 ids");
        names.insert_if_absent(name, id.start)
    }

    fn pools(&self) -> Result<MutexGuard<'_, HashMap<Pool, Names>>, StoreError> {
        lock(&self.pools, "dictionary")
    }
}

impl Dictionary for MemoryDictionary {
    async fn lookup(&self, pool: Pool, name: &str) -> Result<Option<u32>, StoreError> {
        let pools = self.pools()?;
        Ok(pools
            .get(&pool)
            .and_then(|names| names.ids.get(name))
            .copied())
    }

    async fn name(&self, pool: Pool, id: u32) -> Result<Option<String>, StoreError> {
        let pools = self.pools()?;
        Ok(pools
            .get(&pool)
            .and_then(|names| names.names.get(&id))
            .cloned())
    }
}

impl NameStore for MemoryDictionary {
    async fn insert_if_absent(&self, pool: Pool, name: &str, id: u32) -> Result<u32, StoreError> {
        Ok(self
            .pools()?
            .entry(pool)
            .or_default()
            .insert_if_absent(name, id))
    }

    async fn lease(&self, pool: Pool, count: u32) -> Result<Range<u32>, StoreError> {
        Ok(self.pools()?.entry(pool).or_default().lease(count))
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
            match *write {
                FactWrite::Insert(fact) => {
                    let spans = self
                        .spans
                        .entry(fact.subjectset)
                        .or_default()
                        .entry(fact.subject)
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

    fn contains(&self, set: Subjectset, subject: Subject, revision: Revision) -> bool {
        self.spans
            .get(&set)
            .and_then(|subjects| subjects.get(&subject))
            .is_some_and(|spans| spans.iter().any(|span| span.covers(revision)))
    }

    fn visible(&self, set: Subjectset, revision: Revision) -> impl Iterator<Item = Subject> + '_ {
        self.spans
            .get(&set)
            .into_iter()
            .flatten()
            .filter(move |(_, spans)| spans.iter().any(|span| span.covers(revision)))
            .map(|(subject, _)| *subject)
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
        let mut facts = self.facts.lock().expect("fact store poisoned");
        facts.write(&[FactWrite::Insert(fact)])
    }

    fn facts(&self) -> Result<MutexGuard<'_, Facts>, StoreError> {
        lock(&self.facts, "fact store")
    }
}

impl FactStore for MemoryFactStore {
    async fn head(&self) -> Result<Revision, StoreError> {
        Ok(Revision(self.facts()?.head))
    }

    async fn contains(
        &self,
        set: Subjectset,
        subject: Subject,
        revision: Revision,
    ) -> Result<bool, StoreError> {
        Ok(self.facts()?.contains(set, subject, revision))
    }

    async fn subjectsets(
        &self,
        set: Subjectset,
        revision: Revision,
    ) -> Result<Vec<Subjectset>, StoreError> {
        Ok(self
            .facts()?
            .visible(set, revision)
            .filter_map(|subject| match subject {
                Subject::Subjectset(nested) => Some(nested),
                _ => None,
            })
            .collect())
    }

    async fn subjects(
        &self,
        set: Subjectset,
        revision: Revision,
    ) -> Result<Vec<Subject>, StoreError> {
        Ok(self.facts()?.visible(set, revision).collect())
    }
}

impl FactWriter for MemoryFactStore {
    async fn write(&self, writes: &[FactWrite]) -> Result<Revision, StoreError> {
        Ok(self.facts()?.write(writes))
    }
}

/// Every theory, by id.
#[derive(Debug, Default)]
pub struct MemoryTheoryStore {
    theories: HashMap<TheoryId, Theory>,
}

impl MemoryTheoryStore {
    /// Declares a theory whole, replacing any earlier declaration.
    pub fn declare(&mut self, id: TheoryId, theory: Theory) {
        self.theories.insert(id, theory);
    }

    /// The theory as declared, `None` when it never was.
    pub fn theory(&self, id: TheoryId) -> Option<&Theory> {
        self.theories.get(&id)
    }
}

impl TheoryStore for MemoryTheoryStore {
    async fn rewrite(
        &self,
        theory: TheoryId,
        relation: RelationId,
    ) -> Result<Option<Arc<Rewrite>>, StoreError> {
        let theory = self.theories.get(&theory);
        Ok(theory.and_then(|theory| theory.rewrite(&relation)).cloned())
    }
}
