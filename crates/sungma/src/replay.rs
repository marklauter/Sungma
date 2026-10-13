//! Replay: judge a recorded decision again at its revision and compare.
//!
//! Facts and namespaces are read at the recorded revision, so later writes,
//! including new namespace versions and retirements, don't affect a
//! replay.

use crate::{
    closure::{Closure, ClosureError},
    decision::{Decision, SEMANTICS, Semantics},
    store::{FactStore, NamespaceStore},
};

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Replay {
    /// The same outcome, on the same grounds.
    Matches,
    /// The same verdict, allowed on other grounds. A store error at
    /// decision time can move the grounds: the walk read past the failed
    /// read and cited another path.
    Regrounded { now: Decision },
    /// The current rules decide differently at the recorded revision.
    Differs { now: Decision },
    /// The decision was made under other evaluation rules, so a difference
    /// may come from the rules rather than the data. `now` is the decision
    /// under the current rules.
    SemanticsChanged { recorded: Semantics, now: Decision },
}

pub async fn replay<T: NamespaceStore + Sync, F: FactStore + Sync>(
    decision: &Decision,
    namespaces: &T,
    facts: &F,
) -> Result<Replay, ClosureError> {
    let now = Closure::new(namespaces, facts, &decision.subjectset, decision.revision)
        .decide(decision.subject.clone())
        .await?;
    Ok(if decision.semantics != SEMANTICS {
        Replay::SemanticsChanged {
            recorded: decision.semantics,
            now,
        }
    } else if now == *decision {
        Replay::Matches
    } else if now.outcome.is_allowed() == decision.outcome.is_allowed() {
        Replay::Regrounded { now }
    } else {
        Replay::Differs { now }
    })
}
