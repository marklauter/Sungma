//! In-memory simulations of the storage ports.

use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use crate::{
    model::{Fact, RelationId, Revision, Subject, Subjectset, TheoryId},
    rewrite::Rewrite,
    store::{Dictionary, FactStore, StoreError, TheoryStore},
    theory::Theory,
};

#[derive(Debug, Default)]
pub struct MemoryDictionary {
    ids: HashMap<String, u32>,
}

impl MemoryDictionary {
    /// Returns the name's id, minting the next one if it's new.
    pub fn intern(&mut self, name: &str) -> u32 {
        let next = u32::try_from(self.ids.len()).expect("dictionary exceeds u32 ids");
        *self.ids.entry(name.to_owned()).or_insert(next)
    }
}

impl Dictionary for MemoryDictionary {
    async fn lookup(&self, name: &str) -> Result<Option<u32>, StoreError> {
        Ok(self.ids.get(name).copied())
    }
}

/// Each insert produces the next revision. Subjects are kept sorted under
/// their subjectset, like sort keys under a partition key, each with the
/// revision it was written at.
#[derive(Debug, Default)]
pub struct MemoryFactStore {
    facts: HashMap<Subjectset, BTreeMap<Subject, Revision>>,
    head: u64,
}

impl MemoryFactStore {
    /// Returns the revision the fact was written at. Rewriting a stored
    /// fact keeps its original revision.
    pub fn insert(&mut self, fact: Fact) -> Revision {
        self.head += 1;
        let written = Revision(self.head);
        *self
            .facts
            .entry(fact.subjectset)
            .or_default()
            .entry(fact.subject)
            .or_insert(written)
    }
}

impl MemoryFactStore {
    fn visible(&self, set: Subjectset, revision: Revision) -> impl Iterator<Item = Subject> + '_ {
        self.facts
            .get(&set)
            .into_iter()
            .flatten()
            .filter(move |(_, written)| **written <= revision)
            .map(|(subject, _)| *subject)
    }
}

impl FactStore for MemoryFactStore {
    async fn head(&self) -> Result<Revision, StoreError> {
        Ok(Revision(self.head))
    }

    async fn contains(
        &self,
        set: Subjectset,
        subject: Subject,
        revision: Revision,
    ) -> Result<bool, StoreError> {
        let written = self
            .facts
            .get(&set)
            .and_then(|subjects| subjects.get(&subject));
        Ok(written.is_some_and(|written| *written <= revision))
    }

    async fn subjectsets(
        &self,
        set: Subjectset,
        revision: Revision,
    ) -> Result<Vec<Subjectset>, StoreError> {
        Ok(self
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
        Ok(self.visible(set, revision).collect())
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
