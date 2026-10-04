//! Minting ids across nodes.
//!
//! Each node leases a block of ids from the store's shared counter and
//! mints from it locally, so the counter is touched once per block rather
//! than once per name. A new name is stored with a conditional insert: when
//! two nodes intern the same name at once, the first insert wins, and the
//! other node takes the winner's id and skips its own. Skipped ids and the
//! unused rest of a block when a node stops are gaps, which ids may have.

use std::{
    ops::Range,
    sync::{Mutex, PoisonError},
};

use crate::store::{Dictionary, Interner, NameStore, StoreError};

/// An [`Interner`] over a [`NameStore`], minting from leased blocks.
#[derive(Debug)]
pub struct LeasingInterner<S> {
    store: S,
    block: u32,
    lease: Mutex<Range<u32>>,
}

impl<S> LeasingInterner<S> {
    /// Leases `block` ids at a time.
    pub fn new(store: S, block: u32) -> Self {
        Self {
            store,
            block,
            lease: Mutex::new(0..0),
        }
    }

    /// The next id of the current block, `None` once it's spent. A
    /// poisoned lock still holds a whole range, so it's used as is.
    fn take(&self) -> Option<u32> {
        self.lease
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .next()
    }
}

impl<S: NameStore + Sync> LeasingInterner<S> {
    /// Takes the next id, leasing a new block when the current one is spent.
    /// Two tasks that find it spent at once each lease a block, and the
    /// block left unused is a gap.
    async fn next_id(&self) -> Result<u32, StoreError> {
        if let Some(id) = self.take() {
            return Ok(id);
        }
        let mut block = self.store.lease(self.block).await?;
        let id = block
            .next()
            .ok_or_else(|| StoreError("no ids left to lease".to_owned()))?;
        *self.lease.lock().unwrap_or_else(PoisonError::into_inner) = block;
        Ok(id)
    }
}

impl<S: NameStore + Sync> Dictionary for LeasingInterner<S> {
    async fn lookup(&self, name: &str) -> Result<Option<u32>, StoreError> {
        self.store.lookup(name).await
    }

    async fn name(&self, id: u32) -> Result<Option<String>, StoreError> {
        self.store.name(id).await
    }
}

impl<S: NameStore + Sync> Interner for LeasingInterner<S> {
    async fn intern(&self, name: &str) -> Result<u32, StoreError> {
        if let Some(id) = self.store.lookup(name).await? {
            return Ok(id);
        }
        let id = self.next_id().await?;
        self.store.insert_if_absent(name, id).await
    }
}
