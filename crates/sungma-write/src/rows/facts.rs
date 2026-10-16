//! Fact writes. A write reads the latest entry of each namespace its
//! inserts name and judges the inserts by their epochs, reads each fact's
//! pointer, and folds the fact's writes into the one change they make. Its
//! unit of work moves the head, checks each latest entry it judged by, and
//! for each fact moves its pointer and closes and opens its spans.

use std::collections::{HashMap, hash_map};

use sungma::{
    graph::{Fact, Subject},
    id::NamespaceId,
    revision::Revision,
};
use sungma_clock::Clock;
use sungma_edge::{
    read::{Consistency, ItemReader},
    write::{ItemWriter, Work},
};
use sungma_schema::{
    facts, head,
    namespaces::{self, Entry},
};

use super::{Failure, RowWriter, Tried, fits, gave_up};
use crate::{
    fact::{check_inserts, opens_span},
    store::{FactWrite, FactWriteError, FactWriter},
};

impl From<Failure> for FactWriteError {
    fn from(failure: Failure) -> Self {
        match failure {
            Failure::Store(error) => error.into(),
            Failure::Clock(fault) => fault.into(),
        }
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
    latest: Option<&sungma::version::Epochs>,
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

/// The rows a fact's change takes: its pointer, and the spans it closes and
/// opens.
fn rows((closed, opened): (bool, bool)) -> usize {
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

impl<R, W, C, N, P> RowWriter<R, W, C, N, P>
where
    R: ItemReader + Sync,
    W: ItemWriter + Sync,
    C: Clock + Sync,
{
    async fn try_write(&self, writes: &[FactWrite]) -> Result<Tried, FactWriteError> {
        let strong = Consistency::Strong;
        let seen_head = head::read(&self.reader, strong).await?;
        let mut latest: Vec<(NamespaceId, Option<Entry>)> = Vec::new();
        for namespace in judged(writes) {
            latest.push((
                namespace,
                namespaces::latest(&self.reader, namespace, strong).await?,
            ));
        }
        let epochs = |namespace: NamespaceId| {
            latest
                .iter()
                .find(|(id, _)| *id == namespace)
                .and_then(|(_, entry)| entry.as_ref()?.epochs())
        };
        check_inserts(writes, epochs)?;
        let mut changes = Vec::new();
        for (fact, inserts) in grouped(writes) {
            let seen =
                facts::read_pointer(&self.reader, &fact.subjectset, &fact.subject, strong).await?;
            let open = seen.flatten();
            let change = fold(
                fact,
                &inserts,
                open,
                epochs(fact.subjectset.resource.namespace),
            );
            changes.push((fact, seen, change));
        }
        let count = 1
            + latest.len()
            + changes
                .iter()
                .map(|(.., change)| rows(*change))
                .sum::<usize>();
        fits(count, self.items.max_steps())?;
        let Some(reading) = self.stamp(seen_head).await? else {
            return Ok(Tried::Lost);
        };
        let at = reading.revision;
        let mut work = Work::new();
        work.push(head::step(seen_head, at));
        for (namespace, seen) in &latest {
            work.push(namespaces::latest_unchanged(*namespace, seen.as_ref()));
        }
        for (fact, seen, (closed, opened)) in changes {
            let (set, of) = (&fact.subjectset, &fact.subject);
            if !closed && !opened {
                work.push(facts::pointer_unchanged(set, of, seen));
                continue;
            }
            if let (true, Some(written)) = (closed, seen.flatten()) {
                work.push(facts::span_closed(set, of, written, at));
            }
            if opened {
                work.push(facts::span_opened(set, of, at));
            }
            work.push(facts::pointer_moved(set, of, seen, opened.then_some(at)));
        }
        Ok(self.commit(work, reading).await?)
    }
}

impl<R, W, C, N, P> FactWriter for RowWriter<R, W, C, N, P>
where
    R: ItemReader + Sync,
    W: ItemWriter + Sync,
    C: Clock + Sync,
    N: Sync,
    P: Sync,
{
    async fn write(&self, writes: &[FactWrite]) -> Result<Revision, FactWriteError> {
        let _writing = self.lock.lock().await;
        for _ in 0..super::ATTEMPTS {
            if let Tried::Written(reading) = self.try_write(writes).await? {
                return Ok(self.settle(reading).await?);
            }
        }
        Err(gave_up().into())
    }
}
