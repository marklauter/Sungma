//! Replay: judge a recorded decision again at its revision and compare.
//!
//! Facts are read at the recorded revision, so later writes don't affect a
//! replay. Theories are not versioned yet: a replay reads the current
//! theories, and a theory change since the decision shows up as
//! [`Replay::Differs`].

use crate::{
    decision::{Decision, SEMANTICS, Semantics},
    extent::{Extent, ExtentError},
    store::FactStore,
    theory::Theories,
};

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Replay {
    /// The same outcome, on the same grounds.
    Matches,
    /// The current rules decide differently at the recorded revision.
    Differs { now: Decision },
    /// The decision was made under other evaluation rules, so a difference
    /// may come from the rules rather than the data. `now` is the decision
    /// under the current rules.
    SemanticsChanged { recorded: Semantics, now: Decision },
}

pub async fn replay<F: FactStore + Sync>(
    decision: &Decision,
    theories: &Theories,
    facts: &F,
) -> Result<Replay, ExtentError> {
    let now = Extent::new(theories, facts, decision.subjectset, decision.revision)
        .decide(decision.subject)
        .await?;
    Ok(if decision.semantics != SEMANTICS {
        Replay::SemanticsChanged {
            recorded: decision.semantics,
            now,
        }
    } else if now == *decision {
        Replay::Matches
    } else {
        Replay::Differs { now }
    })
}
