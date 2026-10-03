//! A theory: the declared relations of one kind of resource, and their
//! rewrites.

use std::collections::HashMap;

use thiserror::Error;

use crate::{
    model::{RelationId, TheoryId},
    rewrite::Rewrite,
};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TheoryError {
    #[error("an empty {0} has no operands")]
    EmptyOperator(&'static str),
}

/// A relation declared without a rewrite is declared with [`Rewrite::This`].
#[derive(Debug, Default)]
pub struct Theory {
    relations: HashMap<RelationId, Rewrite>,
}

/// Every theory Check can reach, by id. A lookup, not a domain concept.
pub type Theories = HashMap<TheoryId, Theory>;

impl Theory {
    pub fn declare(&mut self, relation: RelationId, rewrite: Rewrite) -> Result<(), TheoryError> {
        validate(&rewrite)?;
        self.relations.insert(relation, rewrite);
        Ok(())
    }

    /// `None` when the theory doesn't declare the relation.
    pub fn rewrite(&self, relation: RelationId) -> Option<&Rewrite> {
        self.relations.get(&relation)
    }
}

fn validate(rewrite: &Rewrite) -> Result<(), TheoryError> {
    match rewrite {
        Rewrite::This | Rewrite::Computed(_) | Rewrite::FactTo { .. } => Ok(()),
        Rewrite::Union(operands) if operands.is_empty() => Err(TheoryError::EmptyOperator("union")),
        Rewrite::Intersection(operands) if operands.is_empty() => {
            Err(TheoryError::EmptyOperator("intersection"))
        }
        Rewrite::Union(operands) | Rewrite::Intersection(operands) => {
            operands.iter().try_for_each(validate)
        }
        Rewrite::Exclusion(base, excluded) => {
            validate(base)?;
            validate(excluded)
        }
    }
}
