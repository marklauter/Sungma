//! Interned names: an item per name holding its id, an item per id holding
//! its name, and a counter per pool. See section 5 of the design note.

use std::ops::Range;

use aws_sdk_dynamodb::{
    operation::update_item::UpdateItemError,
    types::{AttributeValue, Put, ReturnValue, TransactWriteItem},
};
use sungma::store::{Pool, StoreError};

use crate::{
    key,
    table::{self, Item, Refused, Table, failed, number},
};

/// How many times an insert that keeps meeting another in flight is tried.
const ATTEMPTS: usize = 16;

const ID: &str = "id";
const NAME: &str = "name";
const NEXT: &str = "next";

impl Table {
    /// The id stored for `name` in `pool`.
    pub(crate) async fn id_of(
        &self,
        pool: Pool,
        name: &str,
        consistent: bool,
    ) -> Result<Option<u32>, StoreError> {
        let Some(item) = self
            .get(key::names(pool), key::name(name), consistent)
            .await?
        else {
            return Ok(None);
        };
        id(&item).map(Some)
    }

    /// The name stored for `id` in `pool`.
    pub(crate) async fn name_of(
        &self,
        pool: Pool,
        id: u32,
        consistent: bool,
    ) -> Result<Option<String>, StoreError> {
        let item = self
            .get(key::names(pool), key::name_id(id), consistent)
            .await?;
        item.map(|item| table::required_string(&item, NAME).map(str::to_owned))
            .transpose()
    }

    /// Stores both items unless the name is stored already, and returns the
    /// name's id afterwards.
    pub(crate) async fn insert_name(
        &self,
        pool: Pool,
        name: &str,
        id: u32,
    ) -> Result<u32, StoreError> {
        let put =
            |sk, attribute: (&str, AttributeValue)| -> Result<TransactWriteItem, StoreError> {
                let mut item = table::key(key::names(pool), sk);
                item.insert(attribute.0.to_owned(), attribute.1);
                let put = Put::builder()
                    .table_name(self.name())
                    .set_item(Some(item))
                    .condition_expression("attribute_not_exists(sk)")
                    .build()
                    .map_err(failed)?;
                Ok(TransactWriteItem::builder().put(put).build())
            };
        for _ in 0..ATTEMPTS {
            let items = vec![
                put(key::name(name), (ID, number(id.into())))?,
                put(key::name_id(id), (NAME, AttributeValue::S(name.to_owned())))?,
            ];
            match self.transact(items).await {
                Ok(()) => return Ok(id),
                Err(Refused::Failed(error)) => return Err(error),
                // Another node's insert of the name is still in flight.
                Err(Refused::Cancelled(reasons))
                    if reasons
                        .iter()
                        .flatten()
                        .any(|reason| reason == "TransactionConflict") =>
                {
                    continue;
                }
                // Another node stored the name first; a consistent read sees it.
                Err(Refused::Cancelled(_)) => {
                    return self.id_of(pool, name, true).await?.ok_or_else(|| {
                        StoreError(format!(
                            "id {id} is taken in {pool:?}, but '{name}' isn't stored"
                        ))
                    });
                }
            }
        }
        Err(StoreError(format!(
            "storing '{name}' lost {ATTEMPTS} races in a row"
        )))
    }

    /// Advances `pool`'s counter by up to `count`, and returns the ids it
    /// passed over. Near the end of the ids it takes what's left.
    pub(crate) async fn lease(&self, pool: Pool, count: u32) -> Result<Range<u32>, StoreError> {
        let limit = u32::MAX - count;
        let added = self
            .client()
            .update_item()
            .table_name(self.name())
            .set_key(Some(table::key(key::names(pool), key::counter())))
            .update_expression("ADD #next :count")
            .condition_expression("attribute_not_exists(#next) OR #next <= :limit")
            .expression_attribute_names("#next", NEXT)
            .expression_attribute_values(":count", number(count.into()))
            .expression_attribute_values(":limit", number(limit.into()))
            .return_values(ReturnValue::UpdatedNew)
            .send()
            .await;
        match added {
            Ok(output) => {
                let next = next(&output.attributes.unwrap_or_default())?;
                Ok(next - count..next)
            }
            Err(error) => match error.into_service_error() {
                UpdateItemError::ConditionalCheckFailedException(_) => self.lease_rest(pool).await,
                other => Err(failed(other)),
            },
        }
    }

    /// Moves the counter to the end of the ids and returns what it passed.
    async fn lease_rest(&self, pool: Pool) -> Result<Range<u32>, StoreError> {
        loop {
            let item = self.get(key::names(pool), key::counter(), true).await?;
            let start = next(&item.unwrap_or_default())?;
            if start == u32::MAX {
                return Ok(start..start);
            }
            let moved = self
                .client()
                .update_item()
                .table_name(self.name())
                .set_key(Some(table::key(key::names(pool), key::counter())))
                .update_expression("SET #next = :end")
                .condition_expression("#next = :start")
                .expression_attribute_names("#next", NEXT)
                .expression_attribute_values(":end", number(u32::MAX.into()))
                .expression_attribute_values(":start", number(start.into()))
                .send()
                .await;
            match moved {
                Ok(_) => return Ok(start..u32::MAX),
                Err(error) => match error.into_service_error() {
                    UpdateItemError::ConditionalCheckFailedException(_) => continue,
                    other => return Err(failed(other)),
                },
            }
        }
    }
}

fn id(item: &Item) -> Result<u32, StoreError> {
    let id = table::required_number(item, ID)?;
    u32::try_from(id).map_err(|_| StoreError(format!("id {id} doesn't fit in 32 bits")))
}

/// The counter's value; a pool never leased from is at 0.
fn next(item: &Item) -> Result<u32, StoreError> {
    let next = table::optional_number(item, NEXT)?.unwrap_or(0);
    u32::try_from(next).map_err(|_| StoreError(format!("counter {next} doesn't fit in 32 bits")))
}
