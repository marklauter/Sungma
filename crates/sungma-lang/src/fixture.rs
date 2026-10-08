//! Fixtures: theories written with readable strings and interned on load,
//! so the core never sees a string.
//!
//! Each theory is a theory document, as [`crate::theory`] describes:
//!
//! ```json
//! { "file": { "owner": "this", "editor": "this | owner" } }
//! ```

use thiserror::Error;

use sungma::{
    memory::{MemoryDictionary, MemoryTheoryStore},
    model::{RelationId, TheoryId},
    store::Pool,
};

use crate::{
    fact::FactError,
    theory::{self, DocumentError},
};

#[derive(Debug, Error)]
pub enum FixtureError {
    #[error("{}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
    Document(Vec<DocumentError>),
    #[error(transparent)]
    Fact(#[from] FactError),
}

/// Declares the theory each document declares, in turn.
pub fn load_theories(
    documents: &[&str],
    dictionary: &MemoryDictionary,
    theories: &mut MemoryTheoryStore,
) -> Result<(), FixtureError> {
    for document in documents {
        let document = theory::parse(document).map_err(FixtureError::Document)?;
        let id = TheoryId(dictionary.intern(Pool::Theories, document.theory.as_str()));
        let relations = document
            .relations
            .map(|name| RelationId(dictionary.intern(Pool::Relations, name.as_str())));
        theories.declare(id, relations);
    }
    Ok(())
}
