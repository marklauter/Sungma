//! Facts: an item per span, keyed by subjectset and then by subject and the
//! revision that wrote the span, and a pointer per fact to its open span.
//! See section 3 of the design note.

use sungma::{
    graph::{Subject, Subjectset},
    revision::Revision,
    store::StoreError,
};

use crate::{
    key,
    namespaces::REVISION,
    table::{self, Query, Table, number},
};

pub(crate) const WRITTEN: &str = "written";
pub(crate) const RETIRED: &str = "retired";
pub(crate) const OPEN: &str = "open";

impl Table {
    /// The revision of the latest write, 0 before the first.
    pub(crate) async fn head(&self, consistent: bool) -> Result<Revision, StoreError> {
        let (pk, sk) = key::head();
        let item = self.get(pk, sk, consistent).await?;
        Ok(item
            .map(|item| table::required_revision(&item, REVISION))
            .transpose()?
            .unwrap_or(Revision(0)))
    }

    /// The subjects stored under `set` at `revision`, only those whose sort
    /// key starts with `prefix` when there is one, each with the revision that wrote its span.
    pub(crate) async fn spans(
        &self,
        set: &Subjectset,
        prefix: Option<Vec<u8>>,
        revision: Revision,
    ) -> Result<Vec<(Subject, Revision)>, StoreError> {
        let mut query = Query::new(key::fact(set));
        query.filter =
            Some("#written <= :at AND (attribute_not_exists(#retired) OR #retired > :at)");
        query.names = vec![("#written", WRITTEN), ("#retired", RETIRED)];
        query.values = vec![(":at", number(revision.0))];
        if let Some(prefix) = prefix {
            query.condition = Some("begins_with(sk, :prefix)");
            query.values.push((":prefix", table::blob(prefix)));
        }
        self.query(query)
            .await?
            .iter()
            .map(|item| {
                let (pk, sk) = table::keys(item);
                key::parse_span(pk, sk)
            })
            .collect()
    }

    /// The revision that wrote the fact's open span, from its pointer.
    pub(crate) async fn open(
        &self,
        pk: &[u8],
        subject: &[u8],
    ) -> Result<Option<Revision>, StoreError> {
        let item = self.get(pk.to_vec(), subject.to_vec(), false).await?;
        Ok(item
            .map(|item| table::optional_revision(&item, OPEN))
            .transpose()?
            .flatten())
    }
}
