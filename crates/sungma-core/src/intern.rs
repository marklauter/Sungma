//! Minting ids across nodes.
//!
//! Each pool has its own shared counter. A node leases a block of ids from
//! it and mints from the block locally, so the counter is touched once per
//! block rather than once per name. A new name is stored with a conditional
//! insert: when two nodes intern the same name at once, the first insert
//! wins, and the other node takes the winner's id and skips its own.
//! Skipped ids and the unused rest of a block when a node stops are gaps,
//! which ids may have.

use std::{
    collections::HashMap,
    num::NonZeroU32,
    ops::Range,
    sync::{Mutex, MutexGuard, PoisonError},
};

use crate::store::{Dictionary, InternError, Interner, NameStore, Pool, StoreError};

/// An [`Interner`] over a [`NameStore`], minting from a leased block per
/// pool.
#[derive(Debug)]
pub struct LeasingInterner<S> {
    store: S,
    block: NonZeroU32,
    leases: Mutex<HashMap<Pool, Range<u32>>>,
}

impl<S> LeasingInterner<S> {
    /// Leases `block` ids at a time.
    pub fn new(store: S, block: NonZeroU32) -> Self {
        Self {
            store,
            block,
            leases: Mutex::default(),
        }
    }

    /// A poisoned lock still holds whole ranges, so they're used as is.
    fn leases(&self) -> MutexGuard<'_, HashMap<Pool, Range<u32>>> {
        self.leases.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The next id of `pool`'s current block, `None` once it's spent.
    fn take(&self, pool: Pool) -> Option<u32> {
        self.leases().get_mut(&pool)?.next()
    }
}

impl<S: NameStore + Sync> LeasingInterner<S> {
    /// Takes the next id, leasing a new block when the current one is spent.
    /// Two tasks that find it spent at once each lease a block, and the
    /// block left unused is a gap.
    async fn next_id(&self, pool: Pool) -> Result<u32, InternError> {
        if let Some(id) = self.take(pool) {
            return Ok(id);
        }
        let mut block = self.store.lease(pool, self.block.get()).await?;
        let id = block.next().ok_or(InternError::Exhausted(pool))?;
        self.leases().insert(pool, block);
        Ok(id)
    }
}

impl<S: NameStore + Sync> Dictionary for LeasingInterner<S> {
    async fn lookup(&self, pool: Pool, name: &str) -> Result<Option<u32>, StoreError> {
        self.store.lookup(pool, name).await
    }

    async fn name(&self, pool: Pool, id: u32) -> Result<Option<String>, StoreError> {
        self.store.name(pool, id).await
    }
}

impl<S: NameStore + Sync> Interner for LeasingInterner<S> {
    async fn intern(&self, pool: Pool, name: &str) -> Result<u32, InternError> {
        if let Some(id) = self.store.lookup(pool, name).await? {
            return Ok(id);
        }
        let id = self.next_id(pool).await?;
        Ok(self.store.insert_if_absent(pool, name, id).await?)
    }
}
