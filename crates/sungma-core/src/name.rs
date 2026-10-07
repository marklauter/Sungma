//! Names as callers write them, parsed once at the edge so nothing past it
//! splits a string.
//!
//! `theory:id` names a resource and `theory:id#relation` a subjectset. A
//! theory name contains no `:` and a relation name no `#`, so the text
//! splits at the first `:` and the last `#`. The resource id between them
//! is opaque and may contain either. No part may be empty, and no relation
//! is named `...`: `theory:id#...` is the resource itself, not a subjectset.
//!
//! Theory and relation names are [`TheoryName`] and [`RelationName`], as
//! `docs/specs/theory-documents.md` specifies. Text from a caller becomes
//! one through [`FromStr`] or [`TryFrom`], which refuse a name outside the
//! grammar; only a trusted source, such as storage, uses `new_unchecked`.

use std::{
    fmt::{self, Write},
    str::FromStr,
};

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum NameError {
    #[error("malformed resource '{0}', expected theory:id")]
    Resource(String),
    #[error("malformed subjectset '{0}', expected theory:id#relation")]
    Subjectset(String),
    #[error("'{0}' names a resource, not a subjectset")]
    ResourceMember(String),
    #[error("'{0}' is not a valid name")]
    Invalid(String),
    #[error("'{0}' is longer than {MAX_NAME_BYTES} bytes")]
    TooLong(String),
    #[error("'{0}' is reserved")]
    Reserved(String),
}

/// The longest theory or relation name.
pub const MAX_NAME_BYTES: usize = 64;

/// One or more names joined by dots, such as `file` or `drive.file`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct TheoryName(String);

/// One name, never `this` in any casing.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct RelationName(String);

macro_rules! name_type {
    ($name:ident, $check:ident) => {
        impl $name {
            /// A name from a trusted source, such as storage, which holds
            /// only names checked on the way in. Nothing is checked.
            pub fn new_unchecked(name: impl Into<String>) -> Self {
                Self(name.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl FromStr for $name {
            type Err = NameError;

            fn from_str(text: &str) -> Result<Self, NameError> {
                $check(text)?;
                Ok(Self(text.to_owned()))
            }
        }

        impl TryFrom<&str> for $name {
            type Error = NameError;

            fn try_from(text: &str) -> Result<Self, NameError> {
                text.parse()
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

name_type!(TheoryName, check_theory_name);
name_type!(RelationName, check_relation_name);

fn check_theory_name(text: &str) -> Result<(), NameError> {
    if text.len() > MAX_NAME_BYTES {
        Err(NameError::TooLong(quote(text)))
    } else if !text.split('.').all(is_name) {
        Err(NameError::Invalid(quote(text)))
    } else {
        Ok(())
    }
}

fn check_relation_name(text: &str) -> Result<(), NameError> {
    if text.len() > MAX_NAME_BYTES {
        Err(NameError::TooLong(quote(text)))
    } else if !is_name(text) {
        Err(NameError::Invalid(quote(text)))
    } else if is_reserved(text) {
        Err(NameError::Reserved(quote(text)))
    } else {
        Ok(())
    }
}

/// A name of the grammar: a letter or `_`, then letters, digits and `_`.
fn is_name(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `this` in any casing.
fn is_reserved(name: &str) -> bool {
    name.eq_ignore_ascii_case("this")
}

/// The most an error writes of a caller's text, escapes included.
pub const MAX_QUOTE_BYTES: usize = 64;

/// A caller's text as an error quotes it: at most [`MAX_QUOTE_BYTES`]
/// written, escapes included, cut at a character boundary and marked `...` when cut, with control,
/// invisible and reordering characters written as `\u{..}` escapes, so a
/// quote can't forge a log line, recolor a terminal or reorder its line.
pub(crate) fn quote(text: &str) -> String {
    let mut quoted = String::new();
    let mut written = String::new();
    for c in text.chars() {
        written.clear();
        if is_hidden(c) {
            let _ = write!(written, "\\u{{{:04x}}}", u32::from(c));
        } else {
            written.push(c);
        }
        if quoted.len() + written.len() > MAX_QUOTE_BYTES {
            quoted.push_str("...");
            break;
        }
        quoted.push_str(&written);
    }
    quoted
}

/// Control characters, and those that are invisible or reorder text.
fn is_hidden(c: char) -> bool {
    c.is_control()
        || matches!(
            u32::from(c),
            0xAD | 0x34F
                | 0x61C
                | 0x115F..=0x1160
                | 0x17B4..=0x17B5
                | 0x180B..=0x180F
                | 0x200B..=0x200F
                | 0x2028..=0x202E
                | 0x2060..=0x206F
                | 0x3164
                | 0xFE00..=0xFE0F
                | 0xFEFF
                | 0xFFA0
                | 0xFFF0..=0xFFFB
                | 0x1BCA0..=0x1BCA3
                | 0x1D173..=0x1D17A
                | 0xE0000..=0xE0FFF
        )
}

/// `theory:id`
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ResourceName {
    theory: TheoryName,
    id: String,
}

impl ResourceName {
    pub fn theory(&self) -> &TheoryName {
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
                theory: theory.parse()?,
                id: id.to_owned(),
            }),
            _ => Err(NameError::Resource(quote(text))),
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
    relation: RelationName,
}

impl SubjectsetName {
    pub fn resource(&self) -> &ResourceName {
        &self.resource
    }

    pub fn relation(&self) -> &RelationName {
        &self.relation
    }
}

impl FromStr for SubjectsetName {
    type Err = NameError;

    fn from_str(text: &str) -> Result<Self, NameError> {
        let malformed = || NameError::Subjectset(quote(text));
        let (resource, relation) = text.rsplit_once('#').ok_or_else(malformed)?;
        if relation.is_empty() {
            return Err(malformed());
        }
        if relation == "..." {
            return Err(NameError::ResourceMember(quote(text)));
        }
        let resource = resource.parse().map_err(|error| match error {
            NameError::Resource(_) => malformed(),
            name => name,
        })?;
        Ok(Self {
            resource,
            relation: relation.parse()?,
        })
    }
}

impl fmt::Display for SubjectsetName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.resource, self.relation)
    }
}
