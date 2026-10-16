//! The store edge on one DynamoDB table: items under binary hash and range
//! keys, a unit of work as one transaction, and sequences as atomic adds.
//! It knows nothing of what the items mean.

use std::{collections::HashMap, ops::Range as Numbers};

use aws_sdk_dynamodb::{
    error::{DisplayErrorContext, SdkError},
    operation::transact_write_items::TransactWriteItemsError,
    primitives::Blob,
    types::{AttributeValue, ConditionCheck, Put, ReturnValue, TransactWriteItem, Update},
};
use sungma_edge::{
    EdgeError,
    item::{Item, Value},
    key::Key,
    read::{Consistency, ItemReader, Query, Range},
    sequence::Sequence,
    write::{CommitError, Expect, ItemWriter, Step, Work},
};

use crate::table::{PK, SK, Table};

/// DynamoDB commits at most this many items in one transaction.
pub const MAX_STEPS: usize = 100;

/// The attribute a sequence's counter keeps its next number in.
const NEXT: &str = "next";

fn failed(error: impl std::error::Error) -> EdgeError {
    EdgeError(DisplayErrorContext(error).to_string())
}

fn blob(bytes: &[u8]) -> AttributeValue {
    AttributeValue::B(Blob::new(bytes))
}

fn key(key: &Key) -> HashMap<String, AttributeValue> {
    HashMap::from([
        (PK.to_owned(), blob(&key.partition)),
        (SK.to_owned(), blob(&key.sort)),
    ])
}

fn to_attribute(value: &Value) -> AttributeValue {
    match value {
        Value::Number(number) => AttributeValue::N(number.to_string()),
        Value::Text(text) => AttributeValue::S(text.clone()),
        Value::Flag(flag) => AttributeValue::Bool(*flag),
        Value::Map(map) => AttributeValue::M(
            map.iter()
                .map(|(name, value)| (name.clone(), to_attribute(value)))
                .collect(),
        ),
    }
}

fn from_attribute(name: &str, attribute: &AttributeValue) -> Result<Value, EdgeError> {
    Ok(match attribute {
        AttributeValue::N(number) => Value::Number(
            number
                .parse()
                .map_err(|_| EdgeError(format!("attribute '{name}' isn't a whole number")))?,
        ),
        AttributeValue::S(text) => Value::Text(text.clone()),
        AttributeValue::Bool(flag) => Value::Flag(*flag),
        AttributeValue::M(map) => Value::Map(
            map.iter()
                .map(|(name, value)| Ok((name.clone(), from_attribute(name, value)?)))
                .collect::<Result<_, EdgeError>>()?,
        ),
        _ => {
            return Err(EdgeError(format!(
                "attribute '{name}' is of a kind the edge doesn't keep"
            )));
        }
    })
}

/// An item's attributes, without its keys.
fn from_item(item: &HashMap<String, AttributeValue>) -> Result<Item, EdgeError> {
    let mut read = Item::new();
    for (name, attribute) in item {
        if name != PK && name != SK {
            read.0
                .insert(name.clone(), from_attribute(name, attribute)?);
        }
    }
    Ok(read)
}

fn sort_key(item: &HashMap<String, AttributeValue>) -> Result<Vec<u8>, EdgeError> {
    match item.get(SK) {
        Some(AttributeValue::B(bytes)) => Ok(bytes.as_ref().to_vec()),
        _ => Err(EdgeError("an item without a binary sort key".to_owned())),
    }
}

/// A condition expression and the names and values it refers to.
#[derive(Default)]
struct Expression {
    clauses: Vec<String>,
    names: HashMap<String, String>,
    values: HashMap<String, AttributeValue>,
}

impl Expression {
    fn clause(&mut self, clause: String) {
        self.clauses.push(clause);
    }

    fn name(&mut self, name: &str) -> String {
        let alias = format!("#a{}", self.names.len());
        self.names.insert(alias.clone(), name.to_owned());
        alias
    }

    fn value(&mut self, value: &Value) -> String {
        let alias = format!(":v{}", self.values.len());
        self.values.insert(alias.clone(), to_attribute(value));
        alias
    }

    fn expect(&mut self, expect: &[Expect]) {
        for expect in expect {
            let clause = match expect {
                Expect::Equals(name, value) => {
                    format!("{} = {}", self.name(name), self.value(value))
                }
                Expect::Missing(name) => format!("attribute_not_exists({})", self.name(name)),
            };
            self.clause(clause);
        }
    }

    fn condition(&self) -> String {
        self.clauses.join(" AND ")
    }

    fn names(&self) -> Option<HashMap<String, String>> {
        (!self.names.is_empty()).then(|| self.names.clone())
    }

    fn values(&self) -> Option<HashMap<String, AttributeValue>> {
        (!self.values.is_empty()).then(|| self.values.clone())
    }
}

/// The edge on `table`.
#[derive(Clone, Debug)]
pub struct DynamoEdge {
    table: Table,
}

impl DynamoEdge {
    pub fn new(table: Table) -> Self {
        Self { table }
    }

    fn transact_item(&self, step: Step) -> Result<TransactWriteItem, EdgeError> {
        let mut expression = Expression::default();
        let exists = |expression: &mut Expression| {
            let pk = expression.name(PK);
            expression.clause(format!("attribute_exists({pk})"));
        };
        let item = match step {
            Step::Insert { key: at, item } => {
                let pk = expression.name(PK);
                expression.clause(format!("attribute_not_exists({pk})"));
                let mut stored = key(&at);
                stored.extend(
                    item.0
                        .iter()
                        .map(|(name, value)| (name.clone(), to_attribute(value))),
                );
                let put = Put::builder()
                    .table_name(self.table.name())
                    .set_item(Some(stored))
                    .condition_expression(expression.condition())
                    .set_expression_attribute_names(expression.names())
                    .build()
                    .map_err(failed)?;
                TransactWriteItem::builder().put(put).build()
            }
            Step::Update {
                key: at,
                set,
                expect,
            } if !set.0.is_empty() => {
                exists(&mut expression);
                expression.expect(&expect);
                let sets: Vec<String> = set
                    .0
                    .iter()
                    .map(|(name, value)| {
                        format!("{} = {}", expression.name(name), expression.value(value))
                    })
                    .collect();
                let update = Update::builder()
                    .table_name(self.table.name())
                    .set_key(Some(key(&at)))
                    .update_expression(format!("SET {}", sets.join(", ")))
                    .condition_expression(expression.condition())
                    .set_expression_attribute_names(expression.names())
                    .set_expression_attribute_values(expression.values())
                    .build()
                    .map_err(failed)?;
                TransactWriteItem::builder().update(update).build()
            }
            // An update that sets nothing only checks.
            Step::Update {
                key: at, expect, ..
            }
            | Step::Check { key: at, expect } => {
                exists(&mut expression);
                expression.expect(&expect);
                self.check(&at, &expression)?
            }
            Step::Absent { key: at } => {
                let pk = expression.name(PK);
                expression.clause(format!("attribute_not_exists({pk})"));
                self.check(&at, &expression)?
            }
        };
        Ok(item)
    }

    fn check(&self, at: &Key, expression: &Expression) -> Result<TransactWriteItem, EdgeError> {
        let check = ConditionCheck::builder()
            .table_name(self.table.name())
            .set_key(Some(key(at)))
            .condition_expression(expression.condition())
            .set_expression_attribute_names(expression.names())
            .set_expression_attribute_values(expression.values())
            .build()
            .map_err(failed)?;
        Ok(TransactWriteItem::builder().condition_check(check).build())
    }
}

impl ItemReader for DynamoEdge {
    async fn get(&self, at: &Key, consistency: Consistency) -> Result<Option<Item>, EdgeError> {
        let output = self
            .table
            .client()
            .get_item()
            .table_name(self.table.name())
            .set_key(Some(key(at)))
            .consistent_read(consistency == Consistency::Strong)
            .send()
            .await
            .map_err(failed)?;
        output.item.as_ref().map(from_item).transpose()
    }

    async fn query(
        &self,
        query: &Query,
        consistency: Consistency,
    ) -> Result<Vec<(Vec<u8>, Item)>, EdgeError> {
        let mut values = HashMap::from([(":pk".to_owned(), blob(&query.partition))]);
        let condition = match &query.range {
            Range::All => "#pk = :pk".to_owned(),
            Range::Prefix(prefix) => {
                values.insert(":prefix".to_owned(), blob(prefix));
                "#pk = :pk AND begins_with(#sk, :prefix)".to_owned()
            }
            Range::Between(first, last) => {
                values.insert(":first".to_owned(), blob(first));
                values.insert(":last".to_owned(), blob(last));
                "#pk = :pk AND #sk BETWEEN :first AND :last".to_owned()
            }
        };
        let mut names = HashMap::from([("#pk".to_owned(), PK.to_owned())]);
        if !matches!(query.range, Range::All) {
            names.insert("#sk".to_owned(), SK.to_owned());
        }
        let limit = query.limit.unwrap_or(usize::MAX);
        let mut items = Vec::new();
        let mut start = None;
        loop {
            let page = i32::try_from(limit - items.len()).ok();
            let output = self
                .table
                .client()
                .query()
                .table_name(self.table.name())
                .key_condition_expression(&condition)
                .set_expression_attribute_names(Some(names.clone()))
                .set_expression_attribute_values(Some(values.clone()))
                .scan_index_forward(!query.descending)
                .consistent_read(consistency == Consistency::Strong)
                .set_limit(page)
                .set_exclusive_start_key(start)
                .send()
                .await
                .map_err(failed)?;
            for item in output.items() {
                items.push((sort_key(item)?, from_item(item)?));
            }
            start = output.last_evaluated_key;
            if start.is_none() || items.len() >= limit {
                return Ok(items);
            }
        }
    }
}

impl ItemWriter for DynamoEdge {
    fn max_steps(&self) -> usize {
        MAX_STEPS
    }

    async fn commit(&self, work: Work) -> Result<(), CommitError> {
        if work.len() > MAX_STEPS {
            return Err(CommitError::Unwritable(format!(
                "a unit of work of {} steps is more than {MAX_STEPS}",
                work.len()
            )));
        }
        let mut keys = std::collections::HashSet::new();
        if let Some(step) = work.0.iter().find(|step| !keys.insert(step.key())) {
            return Err(CommitError::Unwritable(format!(
                "two steps on {:?}",
                step.key()
            )));
        }
        let items = work
            .0
            .into_iter()
            .map(|step| self.transact_item(step))
            .collect::<Result<Vec<_>, _>>()?;
        let result = self
            .table
            .client()
            .transact_write_items()
            .set_transact_items(Some(items))
            .send()
            .await;
        let error = match result {
            Ok(_) => return Ok(()),
            Err(SdkError::ServiceError(error)) => error.into_err(),
            Err(other) => return Err(failed(other).into()),
        };
        let TransactWriteItemsError::TransactionCanceledException(cancelled) = error else {
            return Err(failed(error).into());
        };
        let codes: Vec<Option<&str>> = cancelled
            .cancellation_reasons()
            .iter()
            .map(|reason| reason.code())
            .collect();
        let refused: Vec<usize> = codes
            .iter()
            .enumerate()
            .filter(|(_, code)| **code == Some("ConditionalCheckFailed"))
            .map(|(at, _)| at)
            .collect();
        if !refused.is_empty() {
            return Err(CommitError::Refused(refused));
        }
        if codes.contains(&Some("TransactionConflict")) {
            return Err(CommitError::Contended);
        }
        Err(EdgeError(format!("the transaction was cancelled: {codes:?}")).into())
    }
}

impl Sequence for DynamoEdge {
    async fn next(&self, at: &Key, count: u64) -> Result<Numbers<u64>, EdgeError> {
        let output = self
            .table
            .client()
            .update_item()
            .table_name(self.table.name())
            .set_key(Some(key(at)))
            .update_expression("ADD #next :count")
            .expression_attribute_names("#next", NEXT)
            .expression_attribute_values(":count", AttributeValue::N(count.to_string()))
            .return_values(ReturnValue::UpdatedNew)
            .send()
            .await
            .map_err(failed)?;
        let end: u128 = match output.attributes().and_then(|item| item.get(NEXT)) {
            Some(AttributeValue::N(number)) => number
                .parse()
                .map_err(|_| EdgeError(format!("a counter holds {number}")))?,
            _ => return Err(EdgeError("a counter's add returned no number".to_owned())),
        };
        // A counter keeps counting past what a u64 holds; those numbers
        // are past the end.
        let clamp = |number: u128| u64::try_from(number).unwrap_or(u64::MAX);
        Ok(clamp(end - u128::from(count))..clamp(end))
    }
}
