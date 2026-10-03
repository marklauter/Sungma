//! In-memory simulations of the storage ports.

use std::collections::HashMap;

use crate::{
    model::{Fact, Revision, Subject, Subjectset},
    store::{Dictionary, FactStore, StoreError},
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

/// Each insert produces the next revision.
#[derive(Debug, Default)]
pub struct MemoryFactStore {
    facts: HashMap<Subjectset, Vec<(Revision, Subject)>>,
    head: u64,
}

impl MemoryFactStore {
    /// Returns the revision the fact was written at.
    pub fn insert(&mut self, fact: Fact) -> Revision {
        self.head += 1;
        let written = Revision(self.head);
        self.facts
            .entry(fact.subjectset)
            .or_default()
            .push((written, fact.subject));
        written
    }

    /// The revision of the latest write.
    pub fn head(&self) -> Revision {
        Revision(self.head)
    }
}

impl FactStore for MemoryFactStore {
    async fn subjects(
        &self,
        set: Subjectset,
        revision: Revision,
    ) -> Result<Vec<Subject>, StoreError> {
        let visible = self.facts.get(&set).into_iter().flatten();
        Ok(visible
            .filter(|(written, _)| *written <= revision)
            .map(|(_, subject)| *subject)
            .collect())
    }
}
