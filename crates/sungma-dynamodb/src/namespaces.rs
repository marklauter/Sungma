//! Namespace versions: an item per entry, keyed by the revision it opens
//! at, and a latest item that copies the newest entry. A version is stored
//! as its printed document, minified, with its epochs beside it. See
//! "Namespace versions" in `docs/specs/dynamodb-adapter.md`.

use std::{collections::HashMap, sync::Arc};

use aws_sdk_dynamodb::types::AttributeValue;
use sungma::{
    id::{NamespaceId, RelationId},
    name::{NamespaceName, RelationName},
    namespace::{Namespace, NamespaceError},
    revision::Revision,
    store::{Pool, StoreError},
    version::{Entry, Version},
};
use sungma_lang::namespace::{parse, print};

use crate::{
    key,
    table::{self, Item, Query, Table, number},
};

pub(crate) const REVISION: &str = "revision";
const DOCUMENT: &str = "document";
const EPOCHS: &str = "epochs";
const RETIRED: &str = "retired";

/// The entry a namespace's latest item holds.
pub(crate) struct Latest {
    pub entry: Entry,
}

impl Latest {
    /// Where the latest entry opens: what a write that follows it checks
    /// the latest item still holds.
    pub fn revision(&self) -> Revision {
        self.entry.revision()
    }

    /// The latest entry's version, `None` when it's retired.
    pub fn version(&self) -> Option<&Arc<Version>> {
        match &self.entry {
            Entry::Version(version) => Some(version),
            Entry::Retired(_) => None,
        }
    }
}

/// A rename that can fail as a store does.
struct Failed(StoreError);

impl From<NamespaceError> for Failed {
    fn from(error: NamespaceError) -> Self {
        Self(StoreError(error.to_string()))
    }
}

/// Every relation a namespace names, in its declarations and its rewrites.
fn relations<R: Clone + Eq + std::hash::Hash + std::fmt::Display>(
    namespace: &Namespace<R>,
) -> Result<Vec<R>, StoreError> {
    let mut seen = Vec::new();
    namespace
        .map(|relation| {
            seen.push(relation.clone());
            relation.clone()
        })
        .map_err(|error| StoreError(error.to_string()))?;
    Ok(seen)
}

impl Table {
    /// The namespace's latest item, `None` when it was never written.
    pub(crate) async fn latest(
        &self,
        namespace: NamespaceId,
        consistent: bool,
    ) -> Result<Option<Latest>, StoreError> {
        let item = self
            .get(key::namespace(namespace), key::latest(), consistent)
            .await?;
        let Some(item) = item else {
            return Ok(None);
        };
        Ok(Some(Latest {
            entry: self.entry(&item, consistent).await?,
        }))
    }

    /// The version whose span covers `revision`.
    pub(crate) async fn version_at(
        &self,
        namespace: NamespaceId,
        revision: Revision,
    ) -> Result<Option<Arc<Version>>, StoreError> {
        match self.latest(namespace, false).await? {
            None => return Ok(None),
            Some(latest) if latest.revision() <= revision => {
                return Ok(latest.version().cloned());
            }
            // A newer entry landed after `revision`, so the history answers.
            Some(_) => {}
        }
        let mut query = Query::new(key::namespace(namespace));
        query.condition = Some("sk BETWEEN :first AND :at");
        query.values = vec![
            (":first", table::blob(key::entry(Revision(0)))),
            (":at", table::blob(key::entry(revision))),
        ];
        query.descending = true;
        query.limit = Some(1);
        let Some(item) = self.query(query).await?.pop() else {
            return Ok(None);
        };
        match self.entry(&item, false).await? {
            Entry::Version(version) => Ok(Some(version)),
            Entry::Retired(_) => Ok(None),
        }
    }

    /// The entry an item holds, its document parsed and its names mapped to
    /// ids. A name a read that isn't `consistent` misses is read again
    /// consistently, since names are stored before the versions that use
    /// them.
    async fn entry(&self, item: &Item, consistent: bool) -> Result<Entry, StoreError> {
        let revision = table::required_revision(item, REVISION)?;
        if table::flag(item, RETIRED) {
            return Ok(Entry::Retired(revision));
        }
        let text = table::required_string(item, DOCUMENT)?;
        let document = parse(text).map_err(|errors| {
            StoreError(format!(
                "a stored namespace document is refused: {errors:?}"
            ))
        })?;
        let mut ids = HashMap::new();
        for name in relations(&document.relations)? {
            let mut id = self
                .id_of(Pool::Relations, name.as_str(), consistent)
                .await?;
            if id.is_none() && !consistent {
                id = self.id_of(Pool::Relations, name.as_str(), true).await?;
            }
            let id = id.ok_or_else(|| StoreError(format!("relation '{name}' isn't interned")))?;
            ids.insert(name, RelationId(id));
        }
        let namespace = document
            .relations
            .try_map(|name| {
                ids.get(name)
                    .copied()
                    .ok_or_else(|| Failed(StoreError(format!("relation '{name}' isn't interned"))))
            })
            .map_err(|Failed(error)| error)?;
        let mut epochs = HashMap::new();
        for (relation, epoch) in table::required_map(item, EPOCHS)? {
            let relation = relation
                .parse()
                .map_err(|_| StoreError(format!("epoch of '{relation}', not a relation id")))?;
            let AttributeValue::N(epoch) = epoch else {
                return Err(StoreError(format!("epoch of {relation} isn't a number")));
            };
            let epoch = epoch
                .parse()
                .map_err(|_| StoreError(format!("epoch of {relation} isn't a revision")))?;
            epochs.insert(RelationId(relation), Revision(epoch));
        }
        let version = Version::stored(namespace, revision, epochs)
            .map_err(|error| StoreError(error.to_string()))?;
        Ok(Entry::Version(Arc::new(version)))
    }

    /// The attributes that store `entry`, beside its key.
    pub(crate) async fn entry_attributes(
        &self,
        namespace: NamespaceId,
        entry: &Entry,
    ) -> Result<Item, StoreError> {
        let mut item = Item::new();
        item.insert(REVISION.to_owned(), number(entry.revision().0));
        let version = match entry {
            Entry::Retired(_) => {
                item.insert(RETIRED.to_owned(), AttributeValue::Bool(true));
                return Ok(item);
            }
            Entry::Version(version) => version,
        };
        item.insert(
            DOCUMENT.to_owned(),
            AttributeValue::S(self.document(namespace, version.namespace()).await?),
        );
        let epochs = version
            .namespace()
            .relations()
            .filter_map(|(relation, _)| {
                let epoch = version.epoch(*relation)?;
                Some((relation.0.to_string(), number(epoch.0)))
            })
            .collect();
        item.insert(EPOCHS.to_owned(), AttributeValue::M(epochs));
        Ok(item)
    }

    /// The namespace printed with its names, minified.
    async fn document(&self, id: NamespaceId, namespace: &Namespace) -> Result<String, StoreError> {
        let unnamed = |what: String| StoreError(format!("{what} has no interned name"));
        let name = self
            .name_of(Pool::Namespaces, id.0, true)
            .await?
            .ok_or_else(|| unnamed(format!("namespace {}", id.0)))?;
        let mut names = HashMap::new();
        for relation in relations(namespace)? {
            let name = self
                .name_of(Pool::Relations, relation.0, true)
                .await?
                .ok_or_else(|| unnamed(format!("relation {}", relation.0)))?;
            names.insert(relation, RelationName::new_unchecked(name));
        }
        let named = namespace
            .try_map(|relation| {
                names
                    .get(relation)
                    .cloned()
                    .ok_or_else(|| Failed(unnamed(format!("relation {}", relation.0))))
            })
            .map_err(|Failed(error)| error)?;
        let text = print(&NamespaceName::new_unchecked(name), &named)
            .map_err(|error| StoreError(error.to_string()))?;
        let json: serde_json::Value =
            serde_json::from_str(&text).map_err(|error| StoreError(error.to_string()))?;
        Ok(json.to_string())
    }
}
