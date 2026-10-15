//! The write ports. Each write is one transaction that also moves the head
//! forward, on condition the head is below the write's revision, and
//! checks every item the write read, so a write that raced another is
//! refused whole and tried again. See section 6 of the design note.

use std::{
    collections::{HashMap, hash_map},
    ops::Range,
    sync::Arc,
};

use aws_sdk_dynamodb::types::{ConditionCheck, Put, TransactWriteItem, Update};
use sungma::{
    graph::{Fact, Subject},
    id::NamespaceId,
    namespace::Namespace,
    revision::Revision,
    store::{Pool, StoreError},
    version::{History, HistoryError, Version},
};
use sungma_clock::{Clock, ClockFault, DevClock, Reading};
use sungma_write::{
    fact::{check_inserts, opens_span},
    store::{
        FactWrite, FactWriteError, FactWriter, NameWriter, NamespaceWrite, NamespaceWriteError,
        NamespaceWriter,
    },
};
use tokio::sync::Mutex;

use crate::{
    facts::{OPEN, RETIRED, WRITTEN},
    key,
    namespaces::{Latest, REVISION},
    table::{self, Item, Refused, Table, failed, number},
};

/// Why sending a transaction failed.
enum SendError {
    Store(StoreError),
    Clock(ClockFault),
}

impl From<StoreError> for SendError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<ClockFault> for SendError {
    fn from(fault: ClockFault) -> Self {
        Self::Clock(fault)
    }
}

impl From<SendError> for FactWriteError {
    fn from(error: SendError) -> Self {
        match error {
            SendError::Store(error) => error.into(),
            SendError::Clock(fault) => fault.into(),
        }
    }
}

impl From<SendError> for NamespaceWriteError {
    fn from(error: SendError) -> Self {
        match error {
            SendError::Store(error) => error.into(),
            SendError::Clock(fault) => fault.into(),
        }
    }
}

/// How many times a write that keeps losing races is tried.
const ATTEMPTS: usize = 16;

/// Writes facts, namespace versions and names, stamping facts and versions
/// from one clock. A write takes its reading under a lock, so on one node
/// revisions follow the order writes land in.
#[derive(Debug)]
pub struct DynamoWriter<C> {
    table: Table,
    clock: C,
    lock: Mutex<()>,
}

impl<C> DynamoWriter<C> {
    /// A writer over `table` that stamps its writes from `clock`.
    pub fn new(table: Table, clock: C) -> Self {
        Self {
            table,
            clock,
            lock: Mutex::default(),
        }
    }
}

impl DynamoWriter<DevClock> {
    /// A writer over `table` whose [`DevClock`] starts after the table's
    /// head, so it never issues a revision twice.
    pub async fn dev(table: Table) -> Result<Self, StoreError> {
        let head = table.head(true).await?;
        Ok(Self::new(table, DevClock::starting_after(head)))
    }
}

/// What a write's transaction does to its items, in order: the head first.
struct Transaction<'t> {
    table: &'t Table,
    items: Vec<TransactWriteItem>,
}

impl<'t> Transaction<'t> {
    /// Starts with moving the head to `revision`.
    fn new(table: &'t Table, revision: Revision) -> Result<Self, StoreError> {
        let (pk, sk) = key::head();
        let head = Update::builder()
            .table_name(table.name())
            .set_key(Some(table::key(pk, sk)))
            .update_expression("SET #revision = :new")
            .condition_expression("attribute_not_exists(#revision) OR #revision < :new")
            .expression_attribute_names("#revision", REVISION)
            .expression_attribute_values(":new", number(revision.0))
            .build()
            .map_err(failed)?;
        Ok(Self {
            table,
            items: vec![TransactWriteItem::builder().update(head).build()],
        })
    }

    /// Checks the item still holds `revision` in `attribute`, or doesn't
    /// have it when `None`.
    fn check(
        &mut self,
        pk: Vec<u8>,
        sk: Vec<u8>,
        attribute: &str,
        revision: Option<Revision>,
    ) -> Result<(), StoreError> {
        let (condition, value) = unchanged(revision);
        let check = ConditionCheck::builder()
            .table_name(self.table.name())
            .set_key(Some(table::key(pk, sk)))
            .condition_expression(condition)
            .expression_attribute_names("#attribute", attribute)
            .set_expression_attribute_values(value)
            .build()
            .map_err(failed)?;
        self.items
            .push(TransactWriteItem::builder().condition_check(check).build());
        Ok(())
    }

    /// Stores `attributes` at the key, on condition `attribute` still holds
    /// `revision`, or the item has none when `None`.
    fn put(
        &mut self,
        pk: Vec<u8>,
        sk: Vec<u8>,
        attributes: Item,
        attribute: &str,
        revision: Option<Revision>,
    ) -> Result<(), StoreError> {
        let mut item = table::key(pk, sk);
        item.extend(attributes);
        let (condition, value) = unchanged(revision);
        let put = Put::builder()
            .table_name(self.table.name())
            .set_item(Some(item))
            .condition_expression(condition)
            .expression_attribute_names("#attribute", attribute)
            .set_expression_attribute_values(value)
            .build()
            .map_err(failed)?;
        self.items
            .push(TransactWriteItem::builder().put(put).build());
        Ok(())
    }

    /// Closes the span at the key at `revision`.
    fn retire(&mut self, pk: Vec<u8>, sk: Vec<u8>, revision: Revision) -> Result<(), StoreError> {
        let update = Update::builder()
            .table_name(self.table.name())
            .set_key(Some(table::key(pk, sk)))
            .update_expression("SET #retired = :at")
            .expression_attribute_names("#retired", RETIRED)
            .expression_attribute_values(":at", number(revision.0))
            .build()
            .map_err(failed)?;
        self.items
            .push(TransactWriteItem::builder().update(update).build());
        Ok(())
    }
}

/// The condition that an attribute still holds `revision`, or is absent.
fn unchanged(
    revision: Option<Revision>,
) -> (
    &'static str,
    Option<HashMap<String, aws_sdk_dynamodb::types::AttributeValue>>,
) {
    match revision {
        Some(revision) => (
            "#attribute = :seen",
            Some(HashMap::from([(":seen".to_owned(), number(revision.0))])),
        ),
        None => ("attribute_not_exists(#attribute)", None),
    }
}

/// What applying one fact's writes in order does to a fact whose open span
/// was written at `open`: whether that span closes, and whether a new one
/// opens at the write's revision. A span the batch opens counts, since no
/// epoch is later than the write, so inserting it again changes nothing. A
/// span opened and closed within the batch covers no revision, so it isn't
/// stored.
fn fold(
    fact: &Fact,
    inserts: &[bool],
    open: Option<Revision>,
    latest: Option<&Version>,
) -> (bool, bool) {
    let (mut closed, mut opened) = (false, false);
    for &insert in inserts {
        let before = open.filter(|_| !closed);
        if insert {
            if !opened && opens_span(fact, before, latest) {
                closed |= before.is_some();
                opened = true;
            }
        } else if opened {
            opened = false;
        } else if before.is_some() {
            closed = true;
        }
    }
    (closed, opened)
}

/// The items a fact's change takes: its pointer, and the spans it closes
/// and opens.
fn items((closed, opened): (bool, bool)) -> usize {
    1 + usize::from(closed) + usize::from(opened)
}

/// The namespaces whose latest versions the inserts are judged by.
fn judged(writes: &[FactWrite]) -> Vec<NamespaceId> {
    let mut namespaces = Vec::new();
    for write in writes {
        let FactWrite::Insert(fact) = write else {
            continue;
        };
        namespaces.push(fact.subjectset.resource.namespace);
        if let Subject::Subjectset(subject) = &fact.subject {
            namespaces.push(subject.resource.namespace);
        }
    }
    namespaces.sort();
    namespaces.dedup();
    namespaces
}

/// Each fact's writes, in the order the fact first appears.
fn grouped(writes: &[FactWrite]) -> Vec<(&Fact, Vec<bool>)> {
    let mut facts: Vec<(&Fact, Vec<bool>)> = Vec::new();
    let mut index: HashMap<&Fact, usize> = HashMap::new();
    for write in writes {
        let (fact, insert) = match write {
            FactWrite::Insert(fact) => (fact, true),
            FactWrite::Retire(fact) => (fact, false),
        };
        match index.entry(fact) {
            hash_map::Entry::Occupied(at) => facts[*at.get()].1.push(insert),
            hash_map::Entry::Vacant(at) => {
                at.insert(facts.len());
                facts.push((fact, vec![insert]));
            }
        }
    }
    facts
}

impl<C: Clock + Sync> DynamoWriter<C> {
    /// Sends `transaction`. `Ok(false)` when it lost a race and should be
    /// tried again; when it lost to a later head, the clock moves past it
    /// first.
    async fn send(&self, transaction: Transaction<'_>) -> Result<bool, SendError> {
        match self.table.transact(transaction.items).await {
            Ok(()) => Ok(true),
            Err(Refused::Failed(error)) => Err(error.into()),
            Err(Refused::Cancelled(reasons)) => {
                let head = Some("ConditionalCheckFailed");
                if reasons
                    .first()
                    .is_some_and(|reason| reason.as_deref() == head)
                {
                    self.catch_up(self.table.head(true).await?).await?;
                }
                Ok(false)
            }
        }
    }

    /// Moves the clock past `last`, a revision another writer stamped, so
    /// the next reading comes after it.
    async fn catch_up(&self, last: Revision) -> Result<(), ClockFault> {
        self.clock
            .wait(Reading {
                settled: last,
                revision: last,
            })
            .await
    }

    /// One try at a fact write: `None` when it lost a race. A try after a
    /// lost race reads `consistent`ly, so it sees what beat it.
    async fn try_write(
        &self,
        writes: &[FactWrite],
        consistent: bool,
    ) -> Result<Option<Reading>, FactWriteError> {
        let table = &self.table;
        let mut latest: HashMap<NamespaceId, Option<Latest>> = HashMap::new();
        for namespace in judged(writes) {
            latest.insert(namespace, table.latest(namespace, consistent).await?);
        }
        let version =
            |namespace: NamespaceId| latest.get(&namespace)?.as_ref()?.version().map(Arc::as_ref);
        check_inserts(writes, version)?;
        let mut changes = Vec::new();
        for (fact, inserts) in grouped(writes) {
            let pk = key::fact(&fact.subjectset);
            let subject = key::subject(&fact.subject);
            let open = table.open(&pk, &subject, consistent).await?;
            let latest = version(fact.subjectset.resource.namespace);
            let change = fold(fact, &inserts, open, latest);
            changes.push((pk, subject, open, change));
        }
        let changed: usize = changes.iter().map(|change| items(change.3)).sum();
        let count = 1 + latest.len() + changed;
        if count > table::MAX_TRANSACTION_ITEMS {
            return Err(StoreError(format!(
                "a write of {count} items is more than one transaction holds, {}",
                table::MAX_TRANSACTION_ITEMS
            ))
            .into());
        }
        let reading = self.clock.now()?;
        let at = reading.revision;
        let mut transaction = Transaction::new(table, at)?;
        for (namespace, seen) in &latest {
            let seen = seen.as_ref().map(Latest::revision);
            transaction.check(key::namespace(*namespace), key::latest(), REVISION, seen)?;
        }
        for (pk, subject, open, (closed, opened)) in changes {
            if !closed && !opened {
                transaction.check(pk, subject, OPEN, open)?;
                continue;
            }
            if let (true, Some(written)) = (closed, open) {
                transaction.retire(pk.clone(), key::span(&subject, written), at)?;
            }
            let mut pointer = Item::new();
            if opened {
                let span = Item::from([(WRITTEN.to_owned(), number(at.0))]);
                transaction.put(pk.clone(), key::span(&subject, at), span, WRITTEN, None)?;
                pointer.insert(OPEN.to_owned(), number(at.0));
            }
            transaction.put(pk, subject, pointer, OPEN, open)?;
        }
        Ok(self.send(transaction).await?.then_some(reading))
    }

    /// One try at a namespace write: `None` when it lost a race, or when
    /// a read that wasn't `consistent` found the namespace at another
    /// version, so the next try reads it again before refusing.
    async fn try_write_namespace(
        &self,
        namespace: NamespaceId,
        write: &NamespaceWrite,
        consistent: bool,
    ) -> Result<Option<Reading>, NamespaceWriteError> {
        let table = &self.table;
        let latest = table.latest(namespace, consistent).await?;
        let version = latest
            .as_ref()
            .and_then(Latest::version)
            .map(|version| version.revision());
        let (expected, next) = match write {
            NamespaceWrite::Create(next) => (None, Some(next)),
            NamespaceWrite::NewVersion {
                based_on,
                namespace,
            } => (Some(*based_on), Some(namespace)),
            NamespaceWrite::Retire { .. } if version.is_none() => {
                return Err(HistoryError::NothingToRetire.into());
            }
            NamespaceWrite::Retire { based_on } => (Some(*based_on), None),
        };
        if version != expected {
            if !consistent {
                return Ok(None);
            }
            return Err(NamespaceWriteError::Conflict { latest: version });
        }
        let reading = self.clock.now()?;
        let at = reading.revision;
        let seen = latest.as_ref().map(Latest::revision);
        let mut history = latest
            .map(|latest| History::resume(latest.entry))
            .unwrap_or_default();
        let written = match next {
            Some(next) => history.write(copy(next)?, at),
            None => history.retire(at),
        };
        // The clock is behind the latest entry, which another writer
        // stamped.
        if let Err(HistoryError::NotLater { last, .. }) = written {
            self.catch_up(last).await?;
            return Ok(None);
        }
        written?;
        let Some(entry) = history.entries().last() else {
            return Err(StoreError("a written history has no entry".to_owned()).into());
        };
        let attributes = table.entry_attributes(namespace, entry).await?;
        let mut transaction = Transaction::new(table, at)?;
        let pk = key::namespace(namespace);
        transaction.put(
            pk.clone(),
            key::entry(at),
            attributes.clone(),
            REVISION,
            None,
        )?;
        transaction.put(pk, key::latest(), attributes, REVISION, seen)?;
        Ok(self.send(transaction).await?.then_some(reading))
    }

    async fn settle(&self, reading: Reading) -> Result<Revision, ClockFault> {
        self.clock.wait(reading).await?;
        Ok(reading.revision)
    }
}

/// The same namespace, to store while the caller's stays to try again.
fn copy(namespace: &Namespace) -> Result<Namespace, StoreError> {
    namespace
        .map(|relation| *relation)
        .map_err(|error| StoreError(error.to_string()))
}

fn gave_up() -> StoreError {
    StoreError(format!("the write lost {ATTEMPTS} races in a row"))
}

impl<C: Clock + Sync> FactWriter for DynamoWriter<C> {
    async fn write(&self, writes: &[FactWrite]) -> Result<Revision, FactWriteError> {
        let _writing = self.lock.lock().await;
        for attempt in 0..ATTEMPTS {
            if let Some(reading) = self.try_write(writes, attempt > 0).await? {
                return Ok(self.settle(reading).await?);
            }
        }
        Err(gave_up().into())
    }
}

impl<C: Clock + Sync> NamespaceWriter for DynamoWriter<C> {
    async fn write_namespace(
        &self,
        namespace: NamespaceId,
        write: NamespaceWrite,
    ) -> Result<Revision, NamespaceWriteError> {
        let _writing = self.lock.lock().await;
        for attempt in 0..ATTEMPTS {
            let consistent = attempt > 0;
            if let Some(reading) = self
                .try_write_namespace(namespace, &write, consistent)
                .await?
            {
                return Ok(self.settle(reading).await?);
            }
        }
        Err(gave_up().into())
    }
}

impl<C: Sync> NameWriter for DynamoWriter<C> {
    async fn insert_if_absent(&self, pool: Pool, name: &str, id: u32) -> Result<u32, StoreError> {
        self.table.insert_name(pool, name, id).await
    }

    async fn lease(&self, pool: Pool, count: u32) -> Result<Range<u32>, StoreError> {
        self.table.lease(pool, count).await
    }
}
