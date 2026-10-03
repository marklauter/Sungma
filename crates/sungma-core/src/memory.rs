//! In-memory simulations of the storage ports.

use std::collections::HashMap;

use crate::{
    model::{Fact, Pin, Subject, Subjectset},
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

/// Each insert advances the head pin by one, and a fact is visible at
/// every pin from the one it was written at.
#[derive(Debug, Default)]
pub struct MemoryFactStore {
    facts: HashMap<Subjectset, Vec<(Pin, Subject)>>,
    head: u64,
}

impl MemoryFactStore {
    /// Returns the pin the fact was written at.
    pub fn insert(&mut self, fact: Fact) -> Pin {
        self.head += 1;
        let written = Pin(self.head);
        self.facts
            .entry(fact.subjectset)
            .or_default()
            .push((written, fact.subject));
        written
    }

    /// The pin of the latest write.
    pub fn head(&self) -> Pin {
        Pin(self.head)
    }
}

impl FactStore for MemoryFactStore {
    async fn subjects(&self, set: Subjectset, pin: Pin) -> Result<Vec<Subject>, StoreError> {
        let visible = self.facts.get(&set).into_iter().flatten();
        Ok(visible
            .filter(|(written, _)| *written <= pin)
            .map(|(_, subject)| *subject)
            .collect())
    }
}
