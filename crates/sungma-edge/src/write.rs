//! Writing items: a unit of work that commits whole or not at all.

use std::future::Future;

use thiserror::Error;

use crate::{
    EdgeError,
    item::{Item, Value},
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
    Failed(#[from] EdgeError),
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
