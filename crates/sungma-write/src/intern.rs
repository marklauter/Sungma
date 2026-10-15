//! Minting ids across nodes.
//!
//! Each pool has its own shared counter. A node leases a block of ids from
//! it and mints from the block locally, so the counter is touched once per
//! block rather than once per name. A new name is stored with a conditional
//! insert: when two nodes intern the same name at once, the first insert
//! wins, and the other node takes the winner's id and skips its own.
//! Skipped ids and the unused rest of a block when a node stops are gaps,
//! which ids may have.

use std::{collections::HashMap, num::NonZeroU32, ops::Range};

use parking_lot::{Mutex, MutexGuard};

use sungma::store::{Dictionary, NameReader, Pool, StoreError};

use crate::store::{InternError, Interner, NameWriter};

/// An [`Interner`] that looks names up through a [`NameReader`] and mints
/// through a [`NameWriter`], from a leased block per pool.
#[derive(Debug)]
pub struct LeasingInterner<R, W> {
    reader: R,
    writer: W,
    block: NonZeroU32,
    leases: Mutex<HashMap<Pool, Range<u32>>>,
}

impl<R, W> LeasingInterner<R, W> {
    /// Leases `block` ids at a time.
    pub fn new(reader: R, writer: W, block: NonZeroU32) -> Self {
        Self {
            reader,
            writer,
            block,
            leases: Mutex::default(),
        }
    }

    fn leases(&self) -> MutexGuard<'_, HashMap<Pool, Range<u32>>> {
        self.leases.lock()
    }

    /// The next id of `pool`'s current block, `None` once it's spent.
    fn take(&self, pool: Pool) -> Option<u32> {
        self.leases().get_mut(&pool)?.next()
    }
}

impl<R, W: NameWriter + Sync> LeasingInterner<R, W> {
    /// Takes the next id, leasing a new block when the current one is spent.
    /// Two tasks that find it spent at once each lease a block, and the
    /// block left unused is a gap.
    async fn next_id(&self, pool: Pool) -> Result<u32, InternError> {
        if let Some(id) = self.take(pool) {
            return Ok(id);
        }
        let mut block = self.writer.lease(pool, self.block.get()).await?;
        let id = block.next().ok_or(InternError::Exhausted(pool))?;
        self.leases().insert(pool, block);
        Ok(id)
    }
}

impl<R: NameReader + Sync, W: Sync> Dictionary for LeasingInterner<R, W> {
    async fn lookup(&self, pool: Pool, name: &str) -> Result<Option<u32>, StoreError> {
        self.reader.id(pool, name).await
    }

    async fn name(&self, pool: Pool, id: u32) -> Result<Option<String>, StoreError> {
        self.reader.name(pool, id).await
    }
}

impl<R: NameReader + Sync, W: NameWriter + Sync> Interner for LeasingInterner<R, W> {
    async fn intern(&self, pool: Pool, name: &str) -> Result<u32, InternError> {
        if let Some(id) = self.reader.id(pool, name).await? {
            return Ok(id);
        }
        let id = self.next_id(pool).await?;
        Ok(self.writer.insert_if_absent(pool, name, id).await?)
    }
}
