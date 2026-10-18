//! Check's read ports: facts and namespace versions, each read as of a
//! revision.
//!
//! `fn ... -> impl Future<Output = ...> + Send` is an `async fn` in a trait
//! that also promises its future can move between threads. Implementations
//! still write a plain `async fn`.

use std::{future::Future, sync::Arc};

use sungma::{
    graph::{Subject, Subjectset},
    id::NamespaceId,
    revision::Revision,
    store::StoreError,
    version::Version,
};

/// Facts keyed the way a wide-column store keys them: the subjectset is the
/// partition key and the subject is the sort key. Every read is as of a
/// revision, and gives each fact with the revision that wrote it, where
/// the span covering the read opened. Which facts count is the reader's
/// rule, not the store's: see [`Version::counts`].
pub trait FactReader {
    /// The revision of the latest write, of a fact or of a namespace
    /// version: both share one timeline, so a read at the head sees both.
    fn head(&self) -> impl Future<Output = Result<Revision, StoreError>> + Send;

    /// Point lookup: the revision that wrote `set@subject`, `None` when it
    /// isn't stored at `revision`.
    fn written(
        &self,
        set: &Subjectset,
        subject: &Subject,
        revision: Revision,
    ) -> impl Future<Output = Result<Option<Revision>, StoreError>> + Send;

    /// The subjectsets stored under `set`: a sort-key range read that skips
    /// identities and resource members.
    fn subjectsets(
        &self,
        set: &Subjectset,
        revision: Revision,
    ) -> impl Future<Output = Result<Vec<(Subjectset, Revision)>, StoreError>> + Send;

    /// Every subject stored under `set`.
    fn subjects(
        &self,
        set: &Subjectset,
        revision: Revision,
    ) -> impl Future<Output = Result<Vec<(Subject, Revision)>, StoreError>> + Send;
}

/// Namespace versions, keyed by namespace and then by the revision each
/// opens at. A version is shared, so a cache can evict it while a check
/// still walks it.
pub trait NamespaceReader {
    /// The version whose span covers `revision`, `None` when the namespace
    /// wasn't created yet or was retired then.
    fn version(
        &self,
        namespace: NamespaceId,
        revision: Revision,
    ) -> impl Future<Output = Result<Option<Arc<Version>>, StoreError>> + Send;
}
