//! Reading items: one by its key, or a run of a partition by sort key.

use std::future::Future;

use crate::{TableError, item::Item, key::Key};

/// Whether a read must see every write acknowledged before it. A write
/// reads strongly to check what it depends on; other reads take a
/// snapshot that may lag, which is stale once taken anyway.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Consistency {
    Eventual,
    Strong,
}

/// Which sort keys a query takes within its partition.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Range {
    All,
    /// Sort keys that start with these bytes.
    Prefix(Vec<u8>),
    /// Sort keys from the first to the second, both included.
    Between(Vec<u8>, Vec<u8>),
}

impl Range {
    pub fn contains(&self, sort: &[u8]) -> bool {
        match self {
            Range::All => true,
            Range::Prefix(prefix) => sort.starts_with(prefix),
            Range::Between(first, last) => first.as_slice() <= sort && sort <= last.as_slice(),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Query {
    pub partition: Vec<u8>,
    pub range: Range,
    /// Highest sort key first.
    pub descending: bool,
    /// At most this many items.
    pub limit: Option<usize>,
}

impl Query {
    /// Every item in the partition, in sort key order.
    pub fn partition(partition: Vec<u8>) -> Self {
        Self {
            partition,
            range: Range::All,
            descending: false,
            limit: None,
        }
    }
}

pub trait ItemReader {
    /// The item at `key`, `None` when there's none.
    fn get(
        &self,
        key: &Key,
        consistency: Consistency,
    ) -> impl Future<Output = Result<Option<Item>, TableError>> + Send;

    /// The items the query takes, each with its sort key, in the query's
    /// order.
    fn query(
        &self,
        query: &Query,
        consistency: Consistency,
    ) -> impl Future<Output = Result<Vec<(Vec<u8>, Item)>, TableError>> + Send;
}

impl<R: ItemReader + Sync + ?Sized> ItemReader for &R {
    fn get(
        &self,
        key: &Key,
        consistency: Consistency,
    ) -> impl Future<Output = Result<Option<Item>, TableError>> + Send {
        R::get(self, key, consistency)
    }

    fn query(
        &self,
        query: &Query,
        consistency: Consistency,
    ) -> impl Future<Output = Result<Vec<(Vec<u8>, Item)>, TableError>> + Send {
        R::query(self, query, consistency)
    }
}

impl<R: ItemReader + Send + Sync + ?Sized> ItemReader for std::sync::Arc<R> {
    fn get(
        &self,
        key: &Key,
        consistency: Consistency,
    ) -> impl Future<Output = Result<Option<Item>, TableError>> + Send {
        R::get(self, key, consistency)
    }

    fn query(
        &self,
        query: &Query,
        consistency: Consistency,
    ) -> impl Future<Output = Result<Vec<(Vec<u8>, Item)>, TableError>> + Send {
        R::query(self, query, consistency)
    }
}
