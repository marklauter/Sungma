//! Storing names and leasing ids.

use std::ops::Range;

use sungma::store::{NameReader, Pool, StoreError};
use sungma_edge::{
    item::{Item, Value},
    read::{Consistency, ItemReader},
    sequence::Sequence,
    write::{CommitError, ItemWriter, Step, Work},
};
use sungma_write::store::NameWriter;

use crate::{
    read::NameRows,
    rows::{self, ID_ATTRIBUTE, NAME_ATTRIBUTE, failed},
};

/// How many times an insert that keeps meeting other work in flight is
/// tried.
const ATTEMPTS: usize = 16;

/// Stores a name's two rows in one unit of work, each only if it's new, and
/// leases ids from the pool's counter. Reads after a refusal are strong, so
/// they see what refused it.
#[derive(Debug)]
pub struct NameRowWriter<R, W, S> {
    names: NameRows<R>,
    items: W,
    sequence: S,
}

impl<R, W, S> NameRowWriter<R, W, S> {
    pub fn new(items: R, writer: W, sequence: S) -> Self {
        Self {
            names: NameRows::new(items, Consistency::Strong),
            items: writer,
            sequence,
        }
    }
}

impl<R, W, S> NameWriter for NameRowWriter<R, W, S>
where
    R: ItemReader + Sync,
    W: ItemWriter + Sync,
    S: Sequence + Sync,
{
    async fn insert_if_absent(&self, pool: Pool, name: &str, id: u32) -> Result<u32, StoreError> {
        for _ in 0..ATTEMPTS {
            let work = Work(vec![
                Step::Insert {
                    key: rows::name(pool, name),
                    item: Item::new().with(ID_ATTRIBUTE, Value::Number(u64::from(id))),
                },
                Step::Insert {
                    key: rows::id(pool, id),
                    item: Item::new().with(NAME_ATTRIBUTE, Value::Text(name.to_owned())),
                },
            ]);
            match self.items.commit(work).await {
                Ok(()) => return Ok(id),
                // Another writer stored the name first.
                Err(CommitError::Refused(_)) => {
                    return self.names.id(pool, name).await?.ok_or_else(|| {
                        StoreError(format!(
                            "id {id} of {pool:?} is taken, though name '{name}' isn't stored"
                        ))
                    });
                }
                Err(CommitError::Contended) => continue,
                Err(CommitError::Unwritable(error)) => return Err(StoreError(error)),
                Err(CommitError::Failed(error)) => return Err(failed(error)),
            }
        }
        Err(StoreError(format!(
            "storing name '{name}' met other work {ATTEMPTS} times"
        )))
    }

    /// Ids are `u32`s below `u32::MAX`, so a lease past that takes what's
    /// left, and once there, none.
    async fn lease(&self, pool: Pool, count: u32) -> Result<Range<u32>, StoreError> {
        let leased = self
            .sequence
            .next(&rows::counter(pool), u64::from(count))
            .await
            .map_err(failed)?;
        let id = |number: u64| u32::try_from(number).unwrap_or(u32::MAX);
        Ok(id(leased.start)..id(leased.end))
    }
}
