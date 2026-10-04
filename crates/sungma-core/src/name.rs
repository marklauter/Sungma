//! Names as callers write them, parsed once at the edge so nothing past it
//! splits a string.
//!
//! `theory:id` names a resource and `theory:id#relation` a subjectset. A
//! theory name contains no `:` and a relation name no `#`, so the text
//! splits at the first `:` and the last `#`. The resource id between them
//! is opaque and may contain either. No part may be empty, and no relation
//! is named `...`: `theory:id#...` is the resource itself, not a subjectset.

use std::{fmt, str::FromStr};

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum NameError {
    #[error("malformed resource {0:?}, expected theory:id")]
    Resource(String),
    #[error("malformed subjectset {0:?}, expected theory:id#relation")]
    Subjectset(String),
    #[error("{0:?} names a resource, not a subjectset")]
    ResourceMember(String),
}

/// `theory:id`
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ResourceName {
    theory: String,
    id: String,
}

impl ResourceName {
    pub fn theory(&self) -> &str {
        &self.theory
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

impl FromStr for ResourceName {
    type Err = NameError;

    fn from_str(text: &str) -> Result<Self, NameError> {
        match text.split_once(':') {
            Some((theory, id)) if !theory.is_empty() && !id.is_empty() => Ok(Self {
                theory: theory.to_owned(),
                id: id.to_owned(),
            }),
            _ => Err(NameError::Resource(text.to_owned())),
        }
    }
}

impl fmt::Display for ResourceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.theory, self.id)
    }
}

/// `theory:id#relation`
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SubjectsetName {
    resource: ResourceName,
    relation: String,
}

impl SubjectsetName {
    pub fn resource(&self) -> &ResourceName {
        &self.resource
    }

    pub fn relation(&self) -> &str {
        &self.relation
    }
}

impl FromStr for SubjectsetName {
    type Err = NameError;

    fn from_str(text: &str) -> Result<Self, NameError> {
        let malformed = || NameError::Subjectset(text.to_owned());
        let (resource, relation) = text.rsplit_once('#').ok_or_else(malformed)?;
        if relation.is_empty() {
            return Err(malformed());
        }
        if relation == "..." {
            return Err(NameError::ResourceMember(text.to_owned()));
        }
        Ok(Self {
            resource: resource.parse().map_err(|_| malformed())?,
            relation: relation.to_owned(),
        })
    }
}

impl fmt::Display for SubjectsetName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.resource, self.relation)
    }
}
