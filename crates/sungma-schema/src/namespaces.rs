//! Namespace rows. A namespace is its partition; under it, each entry has
//! a row keyed by the revision it opens at, and a latest row, which sorts
//! first, copies the newest entry. A version's row holds the printed
//! document with names, the canonical form, and the epoch of each relation
//! it declares, keyed by relation id. A retirement's row is marked retired.

use std::collections::{BTreeMap, HashMap};

use sungma::{
    id::{NamespaceId, RelationId},
    revision::Revision,
    store::StoreError,
    version::Epochs,
};
use sungma_edge::{
    item::{Item, Value},
    key::{Key, encode},
    read::{Consistency, ItemReader, Query, Range},
    write::{Expect, Step},
};

use crate::{failed, tags::NAMESPACE};

const LATEST: u8 = 0;
const ENTRY: u8 = 1;

const REVISION: &str = "revision";
const DOCUMENT: &str = "document";
const EPOCHS: &str = "epochs";
const RETIRED: &str = "retired";

pub fn partition(namespace: NamespaceId) -> Vec<u8> {
    encode((NAMESPACE, namespace.0))
}

pub fn latest_key(namespace: NamespaceId) -> Key {
    Key {
        partition: partition(namespace),
        sort: encode(LATEST),
    }
}

pub fn entry_key(namespace: NamespaceId, opens: Revision) -> Key {
    Key {
        partition: partition(namespace),
        sort: encode((ENTRY, opens.0)),
    }
}

/// One entry in a namespace's history, as stored.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Entry {
    Version {
        revision: Revision,
        /// The printed document, with names.
        document: String,
        epochs: Epochs,
    },
    Retired {
        revision: Revision,
    },
}

impl Entry {
    /// Where the entry opens.
    pub fn revision(&self) -> Revision {
        match self {
            Entry::Version { revision, .. } | Entry::Retired { revision } => *revision,
        }
    }

    /// The epochs of a version, `None` for a retirement.
    pub fn epochs(&self) -> Option<&Epochs> {
        match self {
            Entry::Version { epochs, .. } => Some(epochs),
            Entry::Retired { .. } => None,
        }
    }

    fn item(&self) -> Item {
        let item = Item::new().with(REVISION, Value::Number(self.revision().0));
        match self {
            Entry::Retired { .. } => item.with(RETIRED, Value::Flag(true)),
            Entry::Version {
                document, epochs, ..
            } => {
                let epochs: BTreeMap<String, Value> = epochs
                    .iter()
                    .map(|(relation, epoch)| (relation.0.to_string(), Value::Number(epoch.0)))
                    .collect();
                item.with(DOCUMENT, Value::Text(document.clone()))
                    .with(EPOCHS, Value::Map(epochs))
            }
        }
    }

    /// The entry a row holds, refused when Sungma didn't write it.
    pub fn from_item(item: &Item) -> Result<Self, StoreError> {
        let revision = Revision(item.required_number(REVISION).map_err(failed)?);
        if item.flag(RETIRED).map_err(failed)? {
            return Ok(Entry::Retired { revision });
        }
        let document = item.required_text(DOCUMENT).map_err(failed)?.to_owned();
        let mut epochs = HashMap::new();
        for (relation, epoch) in item.required_map(EPOCHS).map_err(failed)? {
            let relation = relation
                .parse()
                .map_err(|_| StoreError(format!("an epoch of '{relation}', not a relation id")))?;
            let Value::Number(epoch) = epoch else {
                return Err(StoreError(format!(
                    "the epoch of {relation} isn't a number"
                )));
            };
            epochs.insert(RelationId(relation), Revision(*epoch));
        }
        Ok(Entry::Version {
            revision,
            document,
            epochs: Epochs::new(epochs),
        })
    }
}

/// The namespace's newest entry, `None` when it was never written. A read
/// that isn't strong may lag: it isn't a promise no newer write is on its
/// way.
pub async fn latest<R: ItemReader>(
    items: &R,
    namespace: NamespaceId,
    consistency: Consistency,
) -> Result<Option<Entry>, StoreError> {
    let row = items
        .get(&latest_key(namespace), consistency)
        .await
        .map_err(failed)?;
    row.as_ref().map(Entry::from_item).transpose()
}

/// The entry whose span covers `revision`, `None` when the namespace had
/// no entry then. The latest row answers when it opens at or before
/// `revision`; otherwise a newer entry landed after it, and the history
/// answers.
pub async fn at<R: ItemReader>(
    items: &R,
    namespace: NamespaceId,
    revision: Revision,
    consistency: Consistency,
) -> Result<Option<Entry>, StoreError> {
    let Some(latest) = latest(items, namespace, consistency).await? else {
        return Ok(None);
    };
    if latest.revision() <= revision {
        return Ok(Some(latest));
    }
    let query = Query {
        partition: partition(namespace),
        range: Range::Between(encode((ENTRY, 0u64)), encode((ENTRY, revision.0))),
        descending: true,
        limit: Some(1),
    };
    let found = items.query(&query, consistency).await.map_err(failed)?;
    found
        .first()
        .map(|(_, item)| Entry::from_item(item))
        .transpose()
}

fn latest_holds(seen: &Entry) -> Expect {
    Expect::Equals(REVISION.to_owned(), Value::Number(seen.revision().0))
}

/// A step that leaves the latest row as `seen`, what the writer read, or
/// refuses.
pub fn latest_unchanged(namespace: NamespaceId, seen: Option<&Entry>) -> Step {
    let key = latest_key(namespace);
    match seen {
        None => Step::Absent { key },
        Some(seen) => Step::Check {
            key,
            expect: vec![latest_holds(seen)],
        },
    }
}

/// The steps that store `entry` and make it the latest, from `seen`.
pub fn written(namespace: NamespaceId, seen: Option<&Entry>, entry: &Entry) -> [Step; 2] {
    let item = entry.item();
    let stored = Step::Insert {
        key: entry_key(namespace, entry.revision()),
        item: item.clone(),
    };
    let key = latest_key(namespace);
    let latest = match seen {
        None => Step::Insert { key, item },
        Some(seen) => Step::Update {
            key,
            remove: [DOCUMENT, EPOCHS, RETIRED]
                .into_iter()
                .filter(|name| item.get(name).is_none())
                .map(str::to_owned)
                .collect(),
            set: item,
            expect: vec![latest_holds(seen)],
        },
    };
    [stored, latest]
}
