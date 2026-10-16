//! Writes over the store edge, into the rows `sungma-schema` lays out.
//! Every write is one unit of work that moves the head, on condition it
//! still holds what the writer read, and checks every row the write
//! depends on, so a write that raced another is refused whole and tried
//! again. A writer handles one write at a time, so its revisions follow
//! the order its writes land in. See `docs/specs/dynamodb-adapter.md`.

mod facts;
mod namespaces;

use sungma::{
    name::{NamespaceName, RelationName},
    namespace::Namespace,
    revision::Revision,
    store::StoreError,
};
use sungma_clock::{Clock, ClockFault, DevClock, Reading};
use sungma_edge::{
    read::{Consistency, ItemReader},
    write::{CommitError, ItemWriter, Work},
};
use sungma_schema::head;
use tokio::sync::Mutex;

/// How many times a write that keeps losing races is tried.
const ATTEMPTS: usize = 16;

/// Prints a namespace version as its canonical document, with names, for
/// the row that stores it. Injected, so the write context needn't know the
/// document language.
pub trait Printer {
    fn print(
        &self,
        namespace: &NamespaceName,
        relations: &Namespace<RelationName>,
    ) -> Result<String, String>;
}

impl<F> Printer for F
where
    F: Fn(&NamespaceName, &Namespace<RelationName>) -> Result<String, String>,
{
    fn print(
        &self,
        namespace: &NamespaceName,
        relations: &Namespace<RelationName>,
    ) -> Result<String, String> {
        self(namespace, relations)
    }
}

/// Writes facts and namespace versions through `items`, reading what they
/// depend on strongly through `reader`, and stamping each write from
/// `clock`. A namespace write prints its version with `printer`, finding
/// names through `names`.
#[derive(Debug)]
pub struct RowWriter<R, W, C, N, P> {
    reader: R,
    items: W,
    clock: C,
    names: N,
    printer: P,
    lock: Mutex<()>,
}

impl<R, W, C, N, P> RowWriter<R, W, C, N, P> {
    pub fn new(reader: R, items: W, clock: C, names: N, printer: P) -> Self {
        Self {
            reader,
            items,
            clock,
            names,
            printer,
            lock: Mutex::default(),
        }
    }
}

impl<R: ItemReader + Sync, W, N, P> RowWriter<R, W, DevClock, N, P> {
    /// A writer whose [`DevClock`] starts after the head `reader` finds, so
    /// a restarted writer doesn't reuse a revision already committed.
    pub async fn dev(reader: R, items: W, names: N, printer: P) -> Result<Self, StoreError> {
        let head = head::read(&reader, Consistency::Strong).await?;
        let clock = DevClock::starting_after(head.unwrap_or(Revision(0)));
        Ok(Self::new(reader, items, clock, names, printer))
    }
}

/// How one try at a write ended.
enum Tried {
    Written(Reading),
    /// It lost a race; try again.
    Lost,
}

/// Why a try failed.
enum Failure {
    Store(StoreError),
    Clock(ClockFault),
}

impl From<StoreError> for Failure {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<ClockFault> for Failure {
    fn from(fault: ClockFault) -> Self {
        Self::Clock(fault)
    }
}

impl<R, W, C, N, P> RowWriter<R, W, C, N, P>
where
    R: ItemReader + Sync,
    W: ItemWriter + Sync,
    C: Clock + Sync,
{
    /// A reading of the clock past `head`, the latest write's revision:
    /// `None` when the clock is behind it, after moving the clock past it,
    /// so the next try can stamp.
    async fn stamp(&self, head: Option<Revision>) -> Result<Option<Reading>, ClockFault> {
        let reading = self.clock.now()?;
        match head {
            Some(head) if reading.revision <= head => {
                self.catch_up(head).await?;
                Ok(None)
            }
            _ => Ok(Some(reading)),
        }
    }

    /// Moves the clock past `last`, a revision another writer stamped.
    async fn catch_up(&self, last: Revision) -> Result<(), ClockFault> {
        self.clock
            .wait(Reading {
                settled: last,
                revision: last,
            })
            .await
    }

    /// Commits `work`, whose first step moves the head from `head`. When
    /// that step was refused, another writer moved the head, so the clock
    /// moves past it before the next try.
    async fn commit(&self, work: Work, reading: Reading) -> Result<Tried, Failure> {
        match self.items.commit(work).await {
            Ok(()) => Ok(Tried::Written(reading)),
            Err(CommitError::Refused(steps)) => {
                if steps.first() == Some(&0)
                    && let Some(head) = head::read(&self.reader, Consistency::Strong).await?
                {
                    self.catch_up(head).await?;
                }
                Ok(Tried::Lost)
            }
            Err(CommitError::Contended) => Ok(Tried::Lost),
            Err(CommitError::Unwritable(error)) => Err(StoreError(error).into()),
            Err(CommitError::Failed(error)) => Err(StoreError(error.0).into()),
        }
    }

    /// Waits out the write's commit-wait, then gives its revision.
    async fn settle(&self, reading: Reading) -> Result<Revision, ClockFault> {
        self.clock.wait(reading).await?;
        Ok(reading.revision)
    }
}

fn gave_up() -> StoreError {
    StoreError(format!("the write lost {ATTEMPTS} races in a row"))
}

/// Refuses a write of more than `most` rows, before the writer takes a
/// reading.
fn fits(steps: usize, most: usize) -> Result<(), StoreError> {
    if steps <= most {
        return Ok(());
    }
    Err(StoreError(format!(
        "a write of {steps} rows is more than one unit of work holds, {most}"
    )))
}
