//! The edge in memory: items in a map sorted by partition and then sort
//! key, behind a lock. Every read is strong, and work commits one unit at
//! a time, so it never meets contention. [`MemorySequence`] keeps a
//! counter per key.

use std::{
    collections::{BTreeMap, HashMap},
    ops::Range,
};

use parking_lot::{Mutex, RwLock};

use crate::{
    EdgeError,
    item::Item,
    key::Key,
    read::{Consistency, ItemReader, Query},
    sequence::Sequence,
    write::{CommitError, ItemWriter, Step, Work},
};

/// As many steps as DynamoDB commits at once, so work that fits here fits
/// there.
pub const MAX_STEPS: usize = 100;

#[derive(Debug, Default)]
pub struct MemoryTable {
    items: RwLock<BTreeMap<Key, Item>>,
}

impl MemoryTable {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ItemReader for MemoryTable {
    async fn get(&self, key: &Key, _: Consistency) -> Result<Option<Item>, EdgeError> {
        Ok(self.items.read().get(key).cloned())
    }

    async fn query(
        &self,
        query: &Query,
        _: Consistency,
    ) -> Result<Vec<(Vec<u8>, Item)>, EdgeError> {
        let items = self.items.read();
        let first = Key {
            partition: query.partition.clone(),
            sort: Vec::new(),
        };
        let run = items
            .range(first..)
            .take_while(|(key, _)| key.partition == query.partition)
            .filter(|(key, _)| query.range.contains(&key.sort))
            .map(|(key, item)| (key.sort.clone(), item.clone()));
        let limit = query.limit.unwrap_or(usize::MAX);
        Ok(if query.descending {
            let mut run: Vec<_> = run.collect();
            run.reverse();
            run.truncate(limit);
            run
        } else {
            run.take(limit).collect()
        })
    }
}

impl ItemWriter for MemoryTable {
    fn max_steps(&self) -> usize {
        MAX_STEPS
    }

    async fn commit(&self, work: Work) -> Result<(), CommitError> {
        work.check(MAX_STEPS)?;
        let mut items = self.items.write();
        let refused: Vec<usize> = work
            .0
            .iter()
            .enumerate()
            .filter(|(_, step)| !step.holds(items.get(step.key())))
            .map(|(at, _)| at)
            .collect();
        if !refused.is_empty() {
            return Err(CommitError::Refused(refused));
        }
        for step in work.0 {
            match step {
                Step::Insert { key, item } => {
                    items.insert(key, item);
                }
                Step::Update {
                    key, set, remove, ..
                } => {
                    if let Some(item) = items.get_mut(&key) {
                        item.0.extend(set.0);
                        for name in remove {
                            item.0.remove(&name);
                        }
                    }
                }
                Step::Check { .. } | Step::Absent { .. } => {}
            }
        }
        Ok(())
    }
}

/// Counters, each handing out the next numbers.
#[derive(Debug, Default)]
pub struct MemorySequence {
    counters: Mutex<HashMap<Key, u64>>,
}

impl MemorySequence {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Sequence for MemorySequence {
    async fn next(&self, key: &Key, count: u64) -> Result<Range<u64>, EdgeError> {
        let mut counters = self.counters.lock();
        let next = counters.entry(key.clone()).or_default();
        let start = *next;
        *next = start.saturating_add(count);
        Ok(start..*next)
    }
}
