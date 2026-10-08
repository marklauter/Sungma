//! A decision: one membership judged at one revision, carrying what a
//! replay needs to judge it again.

use crate::{
    clock::Revision,
    graph::{Fact, Subject, Subjectset},
};

/// A version of the evaluation rules. Bumped whenever a change could make
/// the same facts and theories at the same revision decide differently.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Semantics(pub u32);

/// The evaluation rules this build decides by.
pub const SEMANTICS: Semantics = Semantics(1);

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// `grounds` are the stored facts that establish the membership, in
    /// evaluation order. Rewrite steps that read no fact leave no ground.
    Allowed { grounds: Vec<Fact> },
    /// No derivation exists, so there is nothing to cite.
    Denied,
}

impl Outcome {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Outcome::Allowed { .. })
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Decision {
    pub subjectset: Subjectset,
    pub subject: Subject,
    pub revision: Revision,
    pub semantics: Semantics,
    pub outcome: Outcome,
}
