//! The table and the few calls the adapter makes on it.

use std::{collections::HashMap, sync::Arc};

use aws_sdk_dynamodb::{
    Client,
    error::{DisplayErrorContext, SdkError},
    operation::transact_write_items::TransactWriteItemsError,
    primitives::Blob,
    types::{
        AttributeDefinition, AttributeValue, BillingMode, KeySchemaElement, KeyType,
        ScalarAttributeType, TransactWriteItem,
    },
};
use sungma::{revision::Revision, store::StoreError};

use crate::key;

/// One item's attributes.
pub type Item = HashMap<String, AttributeValue>;

/// The hash key and range key attribute names.
pub const PK: &str = "pk";
pub const SK: &str = "sk";

/// A transaction holds at most this many items.
pub const MAX_TRANSACTION_ITEMS: usize = 100;

/// A failed call to the table.
pub fn failed(error: impl std::error::Error) -> StoreError {
    StoreError(DisplayErrorContext(error).to_string())
}

pub fn blob(bytes: Vec<u8>) -> AttributeValue {
    AttributeValue::B(Blob::new(bytes))
}

pub fn number(number: u64) -> AttributeValue {
    AttributeValue::N(number.to_string())
}

/// The item's key.
pub fn key(pk: Vec<u8>, sk: Vec<u8>) -> Item {
    HashMap::from([(PK.to_owned(), blob(pk)), (SK.to_owned(), blob(sk))])
}

fn bytes<'a>(item: &'a Item, name: &str) -> &'a [u8] {
    match item.get(name) {
        Some(AttributeValue::B(blob)) => blob.as_ref(),
        _ => &[],
    }
}

/// The item's key bytes.
pub fn keys(item: &Item) -> (&[u8], &[u8]) {
    (bytes(item, PK), bytes(item, SK))
}

/// The refusal for an item without an attribute Sungma writes.
fn missing(item: &Item, name: &str) -> StoreError {
    let (pk, sk) = keys(item);
    StoreError(format!(
        "item {} has no valid `{name}`",
        key::describe(pk, sk)
    ))
}

/// A number attribute, `None` when the item doesn't have it.
pub fn optional_number(item: &Item, name: &str) -> Result<Option<u64>, StoreError> {
    match item.get(name) {
        None => Ok(None),
        Some(AttributeValue::N(text)) => text.parse().map(Some).map_err(|_| missing(item, name)),
        Some(_) => Err(missing(item, name)),
    }
}

pub fn required_number(item: &Item, name: &str) -> Result<u64, StoreError> {
    optional_number(item, name)?.ok_or_else(|| missing(item, name))
}

pub fn optional_revision(item: &Item, name: &str) -> Result<Option<Revision>, StoreError> {
    Ok(optional_number(item, name)?.map(Revision))
}

pub fn required_revision(item: &Item, name: &str) -> Result<Revision, StoreError> {
    required_number(item, name).map(Revision)
}

pub fn required_string<'a>(item: &'a Item, name: &str) -> Result<&'a str, StoreError> {
    match item.get(name) {
        Some(AttributeValue::S(text)) => Ok(text),
        _ => Err(missing(item, name)),
    }
}

pub fn required_map<'a>(
    item: &'a Item,
    name: &str,
) -> Result<&'a HashMap<String, AttributeValue>, StoreError> {
    match item.get(name) {
        Some(AttributeValue::M(map)) => Ok(map),
        _ => Err(missing(item, name)),
    }
}

pub fn flag(item: &Item, name: &str) -> bool {
    matches!(item.get(name), Some(AttributeValue::Bool(true)))
}

/// A query: the partition, the sort key condition and filter, and the
/// values they name.
pub struct Query {
    pub pk: Vec<u8>,
    /// Beside `pk = :pk`, such as `begins_with(sk, :prefix)`.
    pub condition: Option<&'static str>,
    pub filter: Option<&'static str>,
    pub names: Vec<(&'static str, &'static str)>,
    pub values: Vec<(&'static str, AttributeValue)>,
    pub descending: bool,
    pub limit: Option<i32>,
}

impl Query {
    pub fn new(pk: Vec<u8>) -> Self {
        Self {
            pk,
            condition: None,
            filter: None,
            names: Vec::new(),
            values: Vec::new(),
            descending: false,
            limit: None,
        }
    }
}

/// Why a transaction wrote nothing.
pub enum Refused {
    /// A condition failed or another transaction held an item. Each reason
    /// is in the order of the transaction's items; `None` where nothing
    /// went wrong.
    Cancelled(Vec<Option<String>>),
    Failed(StoreError),
}

/// A table and the client that reaches it. Cloning shares the client.
#[derive(Clone, Debug)]
pub struct Table {
    client: Client,
    name: Arc<str>,
}

impl Table {
    pub fn new(client: Client, name: impl Into<Arc<str>>) -> Self {
        Self {
            client,
            name: name.into(),
        }
    }

    /// Creates the table: binary hash and range keys, billed per request.
    pub async fn create(&self) -> Result<(), StoreError> {
        let attribute = |name: &str| {
            AttributeDefinition::builder()
                .attribute_name(name)
                .attribute_type(ScalarAttributeType::B)
                .build()
                .map_err(failed)
        };
        let key = |name: &str, kind| {
            KeySchemaElement::builder()
                .attribute_name(name)
                .key_type(kind)
                .build()
                .map_err(failed)
        };
        self.client
            .create_table()
            .table_name(&*self.name)
            .attribute_definitions(attribute(PK)?)
            .attribute_definitions(attribute(SK)?)
            .key_schema(key(PK, KeyType::Hash)?)
            .key_schema(key(SK, KeyType::Range)?)
            .billing_mode(BillingMode::PayPerRequest)
            .send()
            .await
            .map_err(failed)?;
        Ok(())
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    /// The item at the key. A read inside a write after a refusal is
    /// `consistent`, so it sees what refused it.
    pub async fn get(
        &self,
        pk: Vec<u8>,
        sk: Vec<u8>,
        consistent: bool,
    ) -> Result<Option<Item>, StoreError> {
        let output = self
            .client
            .get_item()
            .table_name(&*self.name)
            .set_key(Some(key(pk, sk)))
            .consistent_read(consistent)
            .send()
            .await
            .map_err(failed)?;
        Ok(output.item)
    }

    /// Every item the query matches, page by page.
    pub async fn query(&self, query: Query) -> Result<Vec<Item>, StoreError> {
        let condition = match query.condition {
            Some(condition) => format!("{PK} = :pk AND {condition}"),
            None => format!("{PK} = :pk"),
        };
        let mut values: HashMap<_, _> = query
            .values
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect();
        values.insert(":pk".to_owned(), blob(query.pk));
        let names: HashMap<_, _> = query
            .names
            .into_iter()
            .map(|(alias, name)| (alias.to_owned(), name.to_owned()))
            .collect();
        let mut items = Vec::new();
        let mut start = None;
        loop {
            let output = self
                .client
                .query()
                .table_name(&*self.name)
                .key_condition_expression(&condition)
                .set_filter_expression(query.filter.map(str::to_owned))
                .set_expression_attribute_names((!names.is_empty()).then(|| names.clone()))
                .set_expression_attribute_values(Some(values.clone()))
                .scan_index_forward(!query.descending)
                .set_limit(query.limit)
                .set_exclusive_start_key(start)
                .send()
                .await
                .map_err(failed)?;
            items.extend(output.items.unwrap_or_default());
            // A limited query reads one page.
            let next = output.last_evaluated_key.filter(|_| query.limit.is_none());
            let Some(next) = next else {
                return Ok(items);
            };
            start = Some(next);
        }
    }

    /// Applies `items` atomically, or none of them.
    pub async fn transact(&self, items: Vec<TransactWriteItem>) -> Result<(), Refused> {
        if items.len() > MAX_TRANSACTION_ITEMS {
            return Err(Refused::Failed(StoreError(format!(
                "a write of {} items is more than one transaction holds, {MAX_TRANSACTION_ITEMS}",
                items.len()
            ))));
        }
        let result = self
            .client
            .transact_write_items()
            .set_transact_items(Some(items))
            .send()
            .await;
        match result {
            Ok(_) => Ok(()),
            Err(SdkError::ServiceError(error)) => match error.into_err() {
                TransactWriteItemsError::TransactionCanceledException(cancelled) => {
                    Err(Refused::Cancelled(
                        cancelled
                            .cancellation_reasons()
                            .iter()
                            .map(|reason| {
                                reason
                                    .code()
                                    .filter(|&code| code != "None")
                                    .map(str::to_owned)
                            })
                            .collect(),
                    ))
                }
                other => Err(Refused::Failed(failed(other))),
            },
            Err(other) => Err(Refused::Failed(failed(other))),
        }
    }
}
