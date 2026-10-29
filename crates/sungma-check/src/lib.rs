//! The Check context: resolve a request's names, decide it at a revision,
//! and append an audit record. Check reads facts and namespace versions
//! through its own ports in [`store`], which [`rows::RowReader`] serves
//! from the store edge, and walks rewrites in [`closure`].
//!
//! The [`Decision`] is what replay needs. The [`AuditRecord`] wraps it with
//! what only the edge knows: who asked, when, the names they sent, and the
//! zookie, alongside the revision the decision was made at.
//!
//! The `http` feature adds the Check service's route, in `http`, and the
//! `server` feature its binary, `sungma-check`.

use std::{
    future::Future,
    time::{Duration, Instant, SystemTime},
};

use parking_lot::Mutex;

use thiserror::Error;

use crate::{
    closure::{Closure, ClosureError},
    decision::Decision,
    store::FactReader,
    store::NamespaceReader,
};
use sungma::{
    name::{SubjectName, SubjectsetName},
    revision::Revision,
    store::StoreError,
};
use sungma_names::{
    resolve::{self, ResolveError},
    store::Dictionary,
};

pub mod closure;
pub mod decision;
#[cfg(feature = "http")]
pub mod http;
pub mod replay;
pub mod rows;
pub mod store;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CheckRequest {
    /// Whether the subject is in this subjectset.
    pub set: SubjectsetName,
    pub subject: SubjectName,
    /// Decide at a revision at least this fresh. `None` takes the latest.
    pub zookie: Option<Revision>,
}

/// What the edge knows about a request before it is decided.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RequestContext {
    pub request_id: String,
    pub caller: String,
    pub received_at: SystemTime,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Verdict {
    Decided(Decision),
    /// A namespace or relation name in the request was never interned, so no
    /// fact mentions it and the request is denied without evaluation.
    Unknown,
    /// The request could not be decided.
    Failed(Failure),
}

/// Why a request could not be decided.
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum Failure {
    /// The zookie names a revision past the latest.
    #[error("zookie {zookie:?} is ahead of the latest revision {head:?}")]
    Ahead { zookie: Revision, head: Revision },
    /// A store failed, as its message says.
    #[error("{0}")]
    Store(String),
    /// Evaluation went past one of Sungma's limits.
    #[error("{0}")]
    Evaluation(String),
}

impl From<StoreError> for Failure {
    fn from(error: StoreError) -> Self {
        Self::Store(error.to_string())
    }
}

impl From<ClosureError> for Failure {
    fn from(error: ClosureError) -> Self {
        match error {
            ClosureError::Store(error) => error.into(),
            error => Self::Evaluation(error.to_string()),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AuditRecord {
    pub context: RequestContext,
    pub request: CheckRequest,
    pub elapsed: Duration,
    pub verdict: Verdict,
}

#[derive(Debug, Error)]
pub enum CheckError {
    /// The verdict couldn't be recorded, so it isn't returned.
    #[error("audit append failed: {0}")]
    Audit(StoreError),
}

/// An append-only log of audit records.
pub trait AuditLog {
    fn append(&self, record: AuditRecord) -> impl Future<Output = Result<(), StoreError>> + Send;
}

/// `append` takes `&self`, so the records sit behind a mutex: the log can
/// be shared across concurrent checks and still change.
#[derive(Debug, Default)]
pub struct MemoryAuditLog {
    records: Mutex<Vec<AuditRecord>>,
}

impl MemoryAuditLog {
    pub fn records(&self) -> Vec<AuditRecord> {
        self.records.lock().clone()
    }
}

impl AuditLog for MemoryAuditLog {
    async fn append(&self, record: AuditRecord) -> Result<(), StoreError> {
        self.records.lock().push(record);
        Ok(())
    }
}

pub struct CheckService<'a, D, T, F, A> {
    dictionary: &'a D,
    namespaces: &'a T,
    facts: &'a F,
    audit: &'a A,
}

impl<'a, D, T, F, A> CheckService<'a, D, T, F, A>
where
    D: Dictionary + Sync,
    T: NamespaceReader + Sync,
    F: FactReader + Sync,
    A: AuditLog + Sync,
{
    pub fn new(dictionary: &'a D, namespaces: &'a T, facts: &'a F, audit: &'a A) -> Self {
        Self {
            dictionary,
            namespaces,
            facts,
            audit,
        }
    }

    /// Decides the request and records it. A verdict is returned only once
    /// its record is appended.
    pub async fn check(
        &self,
        context: RequestContext,
        request: CheckRequest,
    ) -> Result<Verdict, CheckError> {
        let started = Instant::now();
        let verdict = self.decide(&request).await;
        let record = AuditRecord {
            context,
            request,
            elapsed: started.elapsed(),
            verdict: verdict.clone(),
        };
        self.audit.append(record).await.map_err(CheckError::Audit)?;
        Ok(verdict)
    }

    async fn decide(&self, request: &CheckRequest) -> Verdict {
        match self.try_decide(request).await {
            Ok(Some(decision)) => Verdict::Decided(decision),
            Ok(None) => Verdict::Unknown,
            Err(failure) => Verdict::Failed(failure),
        }
    }

    /// The latest revision, which must be at least as fresh as the zookie.
    async fn revision(&self, zookie: Option<Revision>) -> Result<Revision, Failure> {
        let head = self.facts.head().await?;
        match zookie {
            Some(zookie) if zookie > head => Err(Failure::Ahead { zookie, head }),
            _ => Ok(head),
        }
    }

    /// `Ok(None)` when a namespace or relation name was never interned.
    async fn try_decide(&self, request: &CheckRequest) -> Result<Option<Decision>, Failure> {
        let names = async {
            let set = resolve::subjectset(self.dictionary, &request.set).await?;
            let subject = resolve::subject(self.dictionary, &request.subject).await?;
            Ok::<_, ResolveError>((set, subject))
        };
        let Some((set, subject)) = resolve::known(names.await)? else {
            return Ok(None);
        };
        let revision = self.revision(request.zookie).await?;
        Ok(Some(
            Closure::new(self.namespaces, self.facts, &set, revision)
                .decide(subject)
                .await?,
        ))
    }
}
