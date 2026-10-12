//! Replay: judge a recorded decision again at its revision and compare.
//!
//! Facts are read at the recorded revision, so later writes don't affect a
//! replay. Namespaces are not versioned yet: a replay reads the current
//! namespaces, and a namespace change since the decision shows up as
//! [`Replay::Differs`] or [`Replay::Regrounded`].

use crate::{
    closure::{Closure, ClosureError},
    decision::{Decision, SEMANTICS, Semantics},
    store::{FactStore, NamespaceStore},
};

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Replay {
    /// The same outcome, on the same grounds.
    Matches,
    /// The same verdict, allowed on other grounds. A namespace change can move
    /// the grounds, and so can a store error at decision time: the walk
    /// read past the failed read and cited another path.
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
    let now = Closure::new(
        namespaces,
        facts,
        decision.subjectset.clone(),
        decision.revision,
    )
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
