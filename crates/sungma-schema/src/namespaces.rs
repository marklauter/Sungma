//! Namespace rows. A namespace is its partition; under it, each entry has
//! a row keyed by the revision it opens at, and a latest row, which sorts
//! first, copies the newest entry. A version's row holds the printed
//! document with names, the canonical form, the epoch of each relation it
//! declares, keyed by relation id, and the constraints of each relation
//! that has any, keyed by relation id, each written with ids: `identity`,
//! `7` or `7#12`. A retirement's row is marked retired.

use std::collections::{BTreeMap, HashMap};

use sungma::{
    id::{NamespaceId, RelationId},
    namespace::Constraint,
    revision::Revision,
    store::StoreError,
    version::{Constraints, Epochs},
};
use sungma_table::{
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
const CONSTRAINTS: &str = "constraints";
/// An identity constraint in a row.
const IDENTITY: &str = "identity";
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
        constraints: Constraints,
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

    /// The constraints of a version, `None` for a retirement.
    pub fn constraints(&self) -> Option<&Constraints> {
        match self {
            Entry::Version { constraints, .. } => Some(constraints),
            Entry::Retired { .. } => None,
        }
    }

    fn item(&self) -> Item {
        let item = Item::new().with(REVISION, Value::Number(self.revision().0));
        match self {
            Entry::Retired { .. } => item.with(RETIRED, Value::Flag(true)),
            Entry::Version {
                document,
                epochs,
                constraints,
                ..
            } => {
                let epochs: BTreeMap<String, Value> = epochs
                    .iter()
                    .map(|(relation, epoch)| (relation.0.to_string(), Value::Number(epoch.0)))
                    .collect();
                let constraints: BTreeMap<String, Value> = constraints
                    .iter()
                    .map(|(relation, constraints)| {
                        let set = constraints
                            .iter()
                            .map(|constraint| (row_constraint(constraint), Value::Flag(true)))
                            .collect();
                        (relation.0.to_string(), Value::Map(set))
                    })
                    .collect();
                item.with(DOCUMENT, Value::Text(document.clone()))
                    .with(EPOCHS, Value::Map(epochs))
                    .with(CONSTRAINTS, Value::Map(constraints))
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
        let mut constraints = HashMap::new();
        for (relation, set) in item.required_map(CONSTRAINTS).map_err(failed)? {
            let id = relation.parse().map_err(|_| {
                StoreError(format!("constraints of '{relation}', not a relation id"))
            })?;
            let Value::Map(set) = set else {
                return Err(StoreError(format!(
                    "the constraints of {relation} aren't a map"
                )));
            };
            let set = set
                .keys()
                .map(|constraint| stored_constraint(constraint))
                .collect::<Result<_, _>>()?;
            constraints.insert(RelationId(id), set);
        }
        Ok(Entry::Version {
            revision,
            document,
            epochs: Epochs::new(epochs),
            constraints: Constraints::new(constraints),
        })
    }
}

/// A constraint as a row writes it with ids.
fn row_constraint(constraint: &Constraint) -> String {
    match constraint {
        Constraint::Identity => IDENTITY.to_owned(),
        Constraint::Resource(namespace) => namespace.0.to_string(),
        Constraint::Subjectset(namespace, relation) => format!("{}#{}", namespace.0, relation.0),
    }
}

/// A constraint as a row writes it with ids, refused when Sungma didn't
/// write it.
fn stored_constraint(text: &str) -> Result<Constraint, StoreError> {
    let id = |text: &str| {
        text.parse::<u32>()
            .map_err(|_| StoreError(format!("'{text}' in a constraint isn't an id")))
    };
    Ok(match text.split_once('#') {
        _ if text == IDENTITY => Constraint::Identity,
        None => Constraint::Resource(NamespaceId(id(text)?)),
        Some((namespace, relation)) => {
            Constraint::Subjectset(NamespaceId(id(namespace)?), RelationId(id(relation)?))
        }
    })
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
            remove: [DOCUMENT, EPOCHS, CONSTRAINTS, RETIRED]
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
