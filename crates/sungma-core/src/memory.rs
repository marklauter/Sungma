//! In-memory simulations of the storage ports.

use std::collections::HashMap;

use crate::{
    model::{Fact, Subject, Subjectset},
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

#[derive(Debug, Default)]
pub struct MemoryFactStore {
    facts: HashMap<Subjectset, Vec<Subject>>,
}

impl MemoryFactStore {
    pub fn insert(&mut self, fact: Fact) {
        self.facts
            .entry(fact.subjectset)
            .or_default()
            .push(fact.subject);
    }
}

impl FactStore for MemoryFactStore {
    async fn subjects(&self, set: Subjectset) -> Result<Vec<Subject>, StoreError> {
        Ok(self.facts.get(&set).cloned().unwrap_or_default())
    }
}
