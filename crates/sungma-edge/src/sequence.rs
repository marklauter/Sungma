//! Sequences: counters that hand out numbers no other caller gets.

use std::{future::Future, ops::Range};

use crate::{EdgeError, key::Key};

pub trait Sequence {
    /// Advances the counter at `key` by `count` and returns the numbers it
    /// passed over, which no other call returns. A counter starts at 0 and
    /// stops at `u64::MAX`: a call past it gets fewer numbers than it asked
    /// for, and once there, none, so an empty range means none are left.
    fn next(
        &self,
        key: &Key,
        count: u64,
    ) -> impl Future<Output = Result<Range<u64>, EdgeError>> + Send;
}

impl<S: Sequence + Sync + ?Sized> Sequence for &S {
    fn next(
        &self,
        key: &Key,
        count: u64,
    ) -> impl Future<Output = Result<Range<u64>, EdgeError>> + Send {
        S::next(self, key, count)
    }
}

impl<S: Sequence + Send + Sync + ?Sized> Sequence for std::sync::Arc<S> {
    fn next(
        &self,
        key: &Key,
        count: u64,
    ) -> impl Future<Output = Result<Range<u64>, EdgeError>> + Send {
        S::next(self, key, count)
    }
}
