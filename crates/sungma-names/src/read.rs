//! Reading names by key, both ways.

use sungma::store::{NameReader, Pool, StoreError};
use sungma_edge::read::{Consistency, ItemReader};

use crate::rows::{self, ID_ATTRIBUTE, NAME_ATTRIBUTE, failed};

/// Reads names from their rows, as consistently as it was made to. A name
/// never changes its id, so a read that lags only misses a name stored
/// moments ago.
#[derive(Clone, Debug)]
pub struct NameRows<R> {
    items: R,
    consistency: Consistency,
}

impl<R> NameRows<R> {
    pub fn new(items: R, consistency: Consistency) -> Self {
        Self { items, consistency }
    }
}

impl<R: ItemReader + Sync> NameReader for NameRows<R> {
    async fn id(&self, pool: Pool, name: &str) -> Result<Option<u32>, StoreError> {
        let row = self
            .items
            .get(&rows::name(pool, name), self.consistency)
            .await
            .map_err(failed)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let id = row.required_number(ID_ATTRIBUTE).map_err(failed)?;
        u32::try_from(id)
            .map(Some)
            .map_err(|_| StoreError(format!("name '{name}' holds id {id}, past every id")))
    }

    async fn name(&self, pool: Pool, id: u32) -> Result<Option<String>, StoreError> {
        let row = self
            .items
            .get(&rows::id(pool, id), self.consistency)
            .await
            .map_err(failed)?;
        row.map(|row| {
            row.required_text(NAME_ATTRIBUTE)
                .map(str::to_owned)
                .map_err(failed)
        })
        .transpose()
    }
}
