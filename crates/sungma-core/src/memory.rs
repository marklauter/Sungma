//! In-memory simulations of the storage ports.

use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex, MutexGuard},
};

use crate::{
    model::{Fact, RelationId, Revision, Subject, Subjectset, TheoryId},
    rewrite::Rewrite,
    store::{Dictionary, FactStore, FactWrite, FactWriter, Interner, StoreError, TheoryStore},
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

#[derive(Debug, Default)]
pub struct MemoryDictionary {
    ids: Mutex<HashMap<String, u32>>,
}

impl MemoryDictionary {
    /// Returns the name's id, minting the next one if it's new.
    pub fn intern(&self, name: &str) -> u32 {
        mint(&mut self.ids.lock().expect("dictionary poisoned"), name)
    }
}

fn mint(ids: &mut HashMap<String, u32>, name: &str) -> u32 {
    let next = u32::try_from(ids.len()).expect("dictionary exceeds u32 ids");
    *ids.entry(name.to_owned()).or_insert(next)
}

impl Dictionary for MemoryDictionary {
    async fn lookup(&self, name: &str) -> Result<Option<u32>, StoreError> {
        Ok(lock(&self.ids, "dictionary")?.get(name).copied())
    }
}

impl Interner for MemoryDictionary {
    async fn intern(&self, name: &str) -> Result<u32, StoreError> {
        let mut ids = lock(&self.ids, "dictionary")?;
        Ok(mint(&mut ids, name))
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
        Ok(theory.and_then(|theory| theory.rewrite(relation)).cloned())
    }
}
