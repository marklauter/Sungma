//! Reading a namespace's version as its stored document.

use sungma::{name::NamespaceName, revision::Revision, store::StoreError};
use sungma_edge::read::ItemReader;
use sungma_names::{resolve, store::Dictionary};
use sungma_schema::namespaces::{self, Entry};

use crate::{READ, ReadError, revision};

/// A version's document as stored, and the revision the version opened at.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NamespaceVersion {
    pub document: String,
    pub revision: Revision,
}

/// The version a read found, `None` when the namespace had none, and the
/// revision it read at.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NamespaceRead {
    pub version: Option<NamespaceVersion>,
    pub revision: Revision,
}

pub struct NamespaceReadService<'a, D, R> {
    dictionary: &'a D,
    items: &'a R,
}

impl<'a, D, R> NamespaceReadService<'a, D, R>
where
    D: Dictionary + Sync,
    R: ItemReader + Sync,
{
    pub fn new(dictionary: &'a D, items: &'a R) -> Self {
        Self { dictionary, items }
    }

    /// The namespace's version at the latest revision, `None` when it was
    /// never created, was retired then, or was never interned.
    pub async fn read(
        &self,
        name: &NamespaceName,
        zookie: Option<Revision>,
    ) -> Result<NamespaceRead, ReadError> {
        let revision = revision(self.items, zookie).await?;
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
        Ok(
            match namespaces::at(self.items, id, revision, READ).await? {
                Some(Entry::Version {
                    revision, document, ..
                }) => Some(NamespaceVersion { document, revision }),
                Some(Entry::Retired { .. }) | None => None,
            },
        )
    }
}
