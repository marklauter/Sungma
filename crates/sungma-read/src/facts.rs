//! Reading the facts under one subjectset.

use sungma::{
    name::{FactName, SubjectName, SubjectsetName},
    revision::Revision,
    store::StoreError,
};
use sungma_names::{resolve, store::Dictionary};
use sungma_schema::{
    facts::{self, Rows},
    namespaces,
};
use sungma_table::read::ItemReader;

use crate::{READ, ReadError, revision};

/// The facts a read found, and the revision it read at.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FactRead {
    pub facts: Vec<FactName>,
    pub revision: Revision,
}

pub struct FactReadService<'a, D, R> {
    dictionary: &'a D,
    items: &'a R,
}

impl<'a, D, R> FactReadService<'a, D, R>
where
    D: Dictionary + Sync,
    R: ItemReader + Sync,
{
    pub fn new(dictionary: &'a D, items: &'a R) -> Self {
        Self { dictionary, items }
    }

    /// The facts under `set` that count at the latest revision, ordered by
    /// subject. With `subject`, only the fact binding it. A name never
    /// interned finds no facts.
    pub async fn read(
        &self,
        set: &SubjectsetName,
        subject: Option<&SubjectName>,
        zookie: Option<Revision>,
    ) -> Result<FactRead, ReadError> {
        let revision = revision(self.items, zookie).await?;
        let facts = self.find(set, subject, revision).await?;
        Ok(FactRead { facts, revision })
    }

    async fn find(
        &self,
        name: &SubjectsetName,
        subject: Option<&SubjectName>,
        revision: Revision,
    ) -> Result<Vec<FactName>, StoreError> {
        let Some(set) = resolve::known(resolve::subjectset(self.dictionary, name).await)? else {
            return Ok(Vec::new());
        };
        let namespace = set.resource.namespace;
        let entry = namespaces::at(self.items, namespace, revision, READ).await?;
        let Some(epochs) = entry.as_ref().and_then(namespaces::Entry::epochs) else {
            return Ok(Vec::new());
        };
        let of = match subject {
            Some(subject) => {
                let found = resolve::subject(self.dictionary, subject).await;
                let Some(id) = resolve::known(found)? else {
                    return Ok(Vec::new());
                };
                Some(id)
            }
            None => None,
        };
        let rows = match &of {
            Some(id) => Rows::Of(id),
            None => Rows::All,
        };
        let rows = facts::rows(self.items, &set, rows, READ).await?;
        let mut found = Vec::new();
        for (held, written) in facts::covering(&rows, revision) {
            if !epochs.counts(set.relation, written) || of.as_ref().is_some_and(|id| *id != held) {
                continue;
            }
            found.push(FactName {
                subjectset: name.clone(),
                subject: resolve::subject_name(self.dictionary, &held).await?,
            });
        }
        Ok(found)
    }
}
