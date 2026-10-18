//! Writing items: a unit of work that commits whole or not at all.

use std::future::Future;

use thiserror::Error;

use crate::{
    TableError,
    item::{Item, PARTITION_KEY, SORT_KEY, Value},
    key::Key,
};

/// What a step expects an attribute of its item to hold when the work
/// commits, as the writer read it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Expect {
    Equals(String, Value),
    Missing(String),
}

impl Expect {
    pub fn holds(&self, item: &Item) -> bool {
        match self {
            Expect::Equals(name, value) => item.get(name) == Some(value),
            Expect::Missing(name) => item.get(name).is_none(),
        }
    }
}

/// One step of a unit of work.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Step {
    /// Stores a new item; refused when one is at the key.
    Insert { key: Key, item: Item },
    /// Sets the attributes in `set` on the item at the key, removes those
    /// named in `remove`, and leaves its others; refused when there's no
    /// item, or one an expectation fails.
    Update {
        key: Key,
        set: Item,
        remove: Vec<String>,
        expect: Vec<Expect>,
    },
    /// Changes nothing; refused when there's no item at the key, or one an
    /// expectation fails.
    Check { key: Key, expect: Vec<Expect> },
    /// Changes nothing; refused when there's an item at the key.
    Absent { key: Key },
}

impl Step {
    pub fn key(&self) -> &Key {
        match self {
            Step::Insert { key, .. }
            | Step::Update { key, .. }
            | Step::Check { key, .. }
            | Step::Absent { key } => key,
        }
    }

    /// Every attribute the step names.
    fn names(&self) -> Vec<&str> {
        fn expected(expect: &[Expect]) -> impl Iterator<Item = &str> {
            expect.iter().map(|expect| match expect {
                Expect::Equals(name, _) | Expect::Missing(name) => name.as_str(),
            })
        }
        match self {
            Step::Insert { item, .. } => item.0.keys().map(String::as_str).collect(),
            Step::Update {
                set,
                remove,
                expect,
                ..
            } => set
                .0
                .keys()
                .map(String::as_str)
                .chain(remove.iter().map(String::as_str))
                .chain(expected(expect))
                .collect(),
            Step::Check { expect, .. } => expected(expect).collect(),
            Step::Absent { .. } => Vec::new(),
        }
    }

    /// Whether the step holds against `stored`, the item at its key.
    pub fn holds(&self, stored: Option<&Item>) -> bool {
        match (self, stored) {
            (Step::Insert { .. } | Step::Absent { .. }, stored) => stored.is_none(),
            (Step::Update { expect, .. } | Step::Check { expect, .. }, Some(item)) => {
                expect.iter().all(|expect| expect.holds(item))
            }
            (Step::Update { .. } | Step::Check { .. }, None) => false,
        }
    }
}

/// Steps that commit together: every one, or none.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Work(pub Vec<Step>);

impl Work {
    pub fn new() -> Self {
        Self::default()
    }

    /// Refuses work no store commits: more than `most` steps, two steps on
    /// one key, or an attribute named as a key attribute.
    pub fn check(&self, most: usize) -> Result<(), CommitError> {
        if self.len() > most {
            return Err(CommitError::Unwritable(format!(
                "a unit of work of {} steps is more than {most}",
                self.len()
            )));
        }
        let mut keys = std::collections::HashSet::new();
        if let Some(step) = self.0.iter().find(|step| !keys.insert(step.key())) {
            return Err(CommitError::Unwritable(format!(
                "two steps on {:?}",
                step.key()
            )));
        }
        let reserved = |name: &str| name == PARTITION_KEY || name == SORT_KEY;
        if let Some(name) = self
            .0
            .iter()
            .flat_map(Step::names)
            .find(|name| reserved(name))
        {
            return Err(CommitError::Unwritable(format!(
                "attribute '{name}' is a key attribute"
            )));
        }
        Ok(())
    }

    pub fn push(&mut self, step: Step) {
        self.0.push(step);
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum CommitError {
    /// These steps, by position, didn't hold, so nothing was written.
    #[error("steps {0:?} didn't hold")]
    Refused(Vec<usize>),
    /// The work met other work in flight; trying again may succeed.
    #[error("the work met other work in flight")]
    Contended,
    /// More steps than the store commits at once, or two on one key.
    #[error("{0}")]
    Unwritable(String),
    #[error(transparent)]
    Failed(#[from] TableError),
}

pub trait ItemWriter {
    /// The most steps one unit of work may hold.
    fn max_steps(&self) -> usize;

    /// Commits every step of `work`, or none of them. A unit of work holds
    /// at most one step per key.
    fn commit(&self, work: Work) -> impl Future<Output = Result<(), CommitError>> + Send;
}

impl<W: ItemWriter + Sync + ?Sized> ItemWriter for &W {
    fn max_steps(&self) -> usize {
        W::max_steps(self)
    }

    fn commit(&self, work: Work) -> impl Future<Output = Result<(), CommitError>> + Send {
        W::commit(self, work)
    }
}

impl<W: ItemWriter + Send + Sync + ?Sized> ItemWriter for std::sync::Arc<W> {
    fn max_steps(&self) -> usize {
        W::max_steps(self)
    }

    fn commit(&self, work: Work) -> impl Future<Output = Result<(), CommitError>> + Send {
        W::commit(self, work)
    }
}
