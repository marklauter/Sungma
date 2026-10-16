//! Fact and namespace reads: what a snapshot holds, named as a caller
//! names it. A read applies the rule every reader applies: a fact counts
//! only when its namespace version at the read declares its relation and
//! the fact was written at or after the relation's epoch. See
//! `docs/specs/one-timeline.md`.

use thiserror::Error;

use sungma::{
    graph::Subject,
    name::{FactName, NamespaceName, SubjectName, SubjectsetName},
    resolve,
    revision::Revision,
    store::{Dictionary, DocumentReader, FactReader, NamespaceReader, StoreError},
};

#[derive(Debug, Error)]
pub enum ReadError {
    /// The zookie names a revision past the latest.
    #[error("zookie {zookie:?} is ahead of the latest revision {head:?}")]
    Ahead { zookie: Revision, head: Revision },
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// The revision a read is made at: the latest, which must be at least as
/// fresh as the zookie.
pub async fn revision<F: FactReader>(
    facts: &F,
    zookie: Option<Revision>,
) -> Result<Revision, ReadError> {
    let head = facts.head().await?;
    match zookie {
        Some(zookie) if zookie > head => Err(ReadError::Ahead { zookie, head }),
        _ => Ok(head),
    }
}

/// The facts a read found, and the revision it read at.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FactRead {
    pub facts: Vec<FactName>,
    pub revision: Revision,
}

/// Reads the facts under one subjectset.
pub struct FactReadService<'a, D, T, F> {
    dictionary: &'a D,
    namespaces: &'a T,
    facts: &'a F,
}

impl<'a, D, T, F> FactReadService<'a, D, T, F>
where
    D: Dictionary + Sync,
    T: NamespaceReader + Sync,
    F: FactReader + Sync,
{
    pub fn new(dictionary: &'a D, namespaces: &'a T, facts: &'a F) -> Self {
        Self {
            dictionary,
            namespaces,
            facts,
        }
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
        let revision = revision(self.facts, zookie).await?;
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
        let Some(version) = self
            .namespaces
            .version(set.resource.namespace, revision)
            .await?
        else {
            return Ok(Vec::new());
        };
        let counts = |written| version.counts(set.relation, written);
        let fact = |subject| FactName {
            subjectset: name.clone(),
            subject,
        };
        if let Some(subject) = subject {
            let Some(id) = resolve::known(resolve::subject(self.dictionary, subject).await)? else {
                return Ok(Vec::new());
            };
            let written = self.facts.written(&set, &id, revision).await?;
            return Ok(written
                .filter(|written| counts(*written))
                .map(|_| fact(subject.clone()))
                .into_iter()
                .collect());
        }
        let mut facts = Vec::new();
        for (subject, written) in self.facts.subjects(&set, revision).await? {
            if counts(written) {
                facts.push(fact(self.subject_name(&subject).await?));
            }
        }
        Ok(facts)
    }

    async fn subject_name(&self, subject: &Subject) -> Result<SubjectName, StoreError> {
        resolve::subject_name(self.dictionary, subject).await
    }
}

/// A namespace version's document as stored, and the revision the version
/// opened at.
#[derive(Debug)]
pub struct NamespaceVersion {
    pub document: String,
    pub revision: Revision,
}

/// The version a read found, `None` when the namespace had none, and the
/// revision it read at.
#[derive(Debug)]
pub struct NamespaceRead {
    pub version: Option<NamespaceVersion>,
    pub revision: Revision,
}

/// Reads one namespace's version as its stored document, for a caller
/// that shows or edits it. Check reads versions another way.
pub struct NamespaceReadService<'a, D, T, F> {
    dictionary: &'a D,
    documents: &'a T,
    facts: &'a F,
}

impl<'a, D, T, F> NamespaceReadService<'a, D, T, F>
where
    D: Dictionary + Sync,
    T: DocumentReader + Sync,
    F: FactReader + Sync,
{
    /// `facts` gives the latest revision, which facts and namespaces share.
    pub fn new(dictionary: &'a D, documents: &'a T, facts: &'a F) -> Self {
        Self {
            dictionary,
            documents,
            facts,
        }
    }

    /// The namespace's version at the latest revision, `None` when it was
    /// never created, was retired then, or was never interned.
    pub async fn read(
        &self,
        name: &NamespaceName,
        zookie: Option<Revision>,
    ) -> Result<NamespaceRead, ReadError> {
        let revision = revision(self.facts, zookie).await?;
        let version = self.find(name, revision).await?;
        Ok(NamespaceRead { version, revision })
    }

    async fn find(
        &self,
        name: &NamespaceName,
        revision: Revision,
    ) -> Result<Option<NamespaceVersion>, StoreError> {
        let Some(id) = resolve::known(resolve::namespace(self.dictionary, name).await)? else {
            return Ok(None);
        };
        let document = self.documents.document(id, revision).await?;
        Ok(document.map(|(revision, document)| NamespaceVersion { document, revision }))
    }
}
