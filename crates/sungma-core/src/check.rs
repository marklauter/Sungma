//! The Check API: resolve a request's names, decide it at a revision, and
//! append an audit record.
//!
//! The [`Decision`] is what replay needs. The [`AuditRecord`] wraps it with
//! what only the edge knows: who asked, when, the names they sent, and the
//! zookie, alongside the revision the decision was made at.

use std::{
    future::Future,
    sync::Mutex,
    time::{Duration, Instant, SystemTime},
};

use thiserror::Error;

use crate::{
    decision::Decision,
    extent::Extent,
    model::{Revision, Subject},
    resolve,
    store::{Dictionary, FactStore, StoreError, TheoryStore},
};

/// A subject named by strings, as a caller sends it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SubjectName {
    Identity(String),
    Subjectset {
        theory: String,
        resource: String,
        relation: String,
    },
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CheckRequest {
    pub theory: String,
    pub resource: String,
    pub relation: String,
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
    /// A name in the request was never interned, so no fact mentions it and
    /// the request is denied without evaluation.
    Unknown,
    /// The request could not be decided.
    Failed(String),
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
        self.records.lock().expect("audit log poisoned").clone()
    }
}

impl AuditLog for MemoryAuditLog {
    async fn append(&self, record: AuditRecord) -> Result<(), StoreError> {
        self.records
            .lock()
            .map_err(|_| StoreError("audit log poisoned".to_owned()))?
            .push(record);
        Ok(())
    }
}

pub struct CheckService<'a, D, T, F, A> {
    dictionary: &'a D,
    theories: &'a T,
    facts: &'a F,
    audit: &'a A,
}

impl<'a, D, T, F, A> CheckService<'a, D, T, F, A>
where
    D: Dictionary + Sync,
    T: TheoryStore + Sync,
    F: FactStore + Sync,
    A: AuditLog + Sync,
{
    pub fn new(dictionary: &'a D, theories: &'a T, facts: &'a F, audit: &'a A) -> Self {
        Self {
            dictionary,
            theories,
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

    /// `Ok(None)` when a name was never interned.
    async fn try_decide(&self, request: &CheckRequest) -> Result<Option<Decision>, String> {
        let Some(set) = resolve::subjectset(
            self.dictionary,
            &request.theory,
            &request.resource,
            &request.relation,
        )
        .await
        .map_err(|e| e.to_string())?
        else {
            return Ok(None);
        };
        let Some(subject) = self.subject(&request.subject).await? else {
            return Ok(None);
        };
        let revision = self.revision(request.zookie).await?;
        Extent::new(self.theories, self.facts, set, revision)
            .decide(subject)
            .await
            .map(Some)
            .map_err(|e| e.to_string())
    }

    async fn subject(&self, name: &SubjectName) -> Result<Option<Subject>, String> {
        let subject = match name {
            SubjectName::Identity(identity) => resolve::identity(self.dictionary, identity)
                .await
                .map(|id| id.map(Subject::Identity)),
            SubjectName::Subjectset {
                theory,
                resource,
                relation,
            } => resolve::subjectset(self.dictionary, theory, resource, relation)
                .await
                .map(|set| set.map(Subject::Subjectset)),
        };
        subject.map_err(|e| e.to_string())
    }

    /// The latest revision, which must be at least as fresh as the zookie.
    async fn revision(&self, zookie: Option<Revision>) -> Result<Revision, String> {
        let head = self.facts.head().await.map_err(|e| e.to_string())?;
        match zookie {
            Some(zookie) if zookie > head => Err(format!(
                "zookie {zookie:?} is ahead of the latest revision {head:?}"
            )),
            _ => Ok(head),
        }
    }
}
