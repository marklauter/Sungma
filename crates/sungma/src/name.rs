//! Names as callers write them, parsed once at the edge so nothing past it
//! splits a string.
//!
//! `namespace:id` names a resource and `namespace:id#relation` a subjectset. A
//! namespace name contains no `:` and a relation name no `#`, so the text
//! splits at the first `:` and the last `#`. The resource id between them
//! is opaque and may contain either. No part may be empty, and no relation
//! is named `...`: `namespace:id#...` is the resource itself, not a subjectset.
//!
//! A resource id and an [`Identity`] are owned by the caller and opaque, and
//! are printable ASCII, `!` to `~`. A resource id is at most
//! [`MAX_RESOURCE_ID_BYTES`] and an identity at most [`MAX_IDENTITY_BYTES`],
//! as `docs/specs/fact-documents.md` specifies.
//!
//! Namespace and relation names are [`NamespaceName`] and [`RelationName`], as
//! `docs/specs/namespace-documents.md` specifies. Text from a caller becomes
//! one through [`FromStr`] or [`TryFrom`], which refuse a name outside the
//! grammar; only a trusted source, such as storage, uses `new_unchecked`.

use std::{
    fmt::{self, Write},
    str::FromStr,
    sync::Arc,
};

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum NameError {
    #[error("malformed resource '{0}', expected namespace:id")]
    Resource(String),
    #[error("malformed subjectset '{0}', expected namespace:id#relation")]
    Subjectset(String),
    #[error("'{0}' names a resource, not a subjectset")]
    ResourceMember(String),
    #[error("'{0}' is not a valid name")]
    Invalid(String),
    #[error("'{0}' is longer than {1} bytes")]
    TooLong(String, usize),
    #[error("'{0}' is reserved")]
    Reserved(String),
    #[error("'{0}' holds a character other than printable ASCII")]
    NotPrintableAscii(String),
    #[error("an identity is empty")]
    EmptyIdentity,
    #[error("a resource is empty")]
    EmptyResource,
    #[error("'{0}' is longer than {MAX_RESOURCE_ID_BYTES} bytes")]
    ResourceIdTooLong(String),
    #[error("'{0}' is longer than {MAX_IDENTITY_BYTES} bytes")]
    IdentityTooLong(String),
}

/// The longest namespace name.
pub const MAX_NAMESPACE_BYTES: usize = 256;

/// The longest relation name.
pub const MAX_RELATION_BYTES: usize = 64;

/// The longest resource id, room enough for a path.
pub const MAX_RESOURCE_ID_BYTES: usize = 512;

/// The longest identity.
pub const MAX_IDENTITY_BYTES: usize = 512;

// Each name holds its text in an `Arc<str>`, so a clone shares the text
// rather than copying it.

/// One or more names joined by dots, such as `file` or `drive.file`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct NamespaceName(Arc<str>);

/// One name, never `this` in any casing.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct RelationName(Arc<str>);

/// A key that references an external principal: opaque to Sungma, never
/// empty, printable ASCII.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Identity(Arc<str>);

/// The id part of a resource name: opaque to Sungma, never empty, at most
/// [`MAX_RESOURCE_ID_BYTES`], printable ASCII.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ResourceId(Arc<str>);

macro_rules! name_type {
    ($name:ident, $check:ident) => {
        impl $name {
            /// A name from a trusted source, such as storage, which holds
            /// only names checked on the way in. Nothing is checked.
            pub fn new_unchecked(name: impl Into<Arc<str>>) -> Self {
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
                Ok(Self(text.into()))
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

name_type!(NamespaceName, check_namespace_name);
name_type!(RelationName, check_relation_name);
name_type!(Identity, check_identity);
name_type!(ResourceId, check_resource_id);

fn check_identity(text: &str) -> Result<(), NameError> {
    if text.is_empty() {
        Err(NameError::EmptyIdentity)
    } else if text.len() > MAX_IDENTITY_BYTES {
        Err(NameError::IdentityTooLong(quote(text)))
    } else {
        check_printable(text)
    }
}

/// Accepts printable ASCII, `!` to `~`, only.
fn check_printable(text: &str) -> Result<(), NameError> {
    if text.bytes().all(|byte| byte.is_ascii_graphic()) {
        Ok(())
    } else {
        Err(NameError::NotPrintableAscii(quote(text)))
    }
}

fn check_namespace_name(text: &str) -> Result<(), NameError> {
    if text.len() > MAX_NAMESPACE_BYTES {
        Err(NameError::TooLong(quote(text), MAX_NAMESPACE_BYTES))
    } else if !text.split('.').all(is_name) {
        Err(NameError::Invalid(quote(text)))
    } else {
        Ok(())
    }
}

fn check_relation_name(text: &str) -> Result<(), NameError> {
    if text.len() > MAX_RELATION_BYTES {
        Err(NameError::TooLong(quote(text), MAX_RELATION_BYTES))
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

/// The most an error writes of a caller's text, escapes and cut mark
/// included.
pub const MAX_QUOTE_BYTES: usize = 64;

/// Marks a quote that was cut.
const CUT: &str = "...";

/// A caller's text as an error quotes it: at most [`MAX_QUOTE_BYTES`]
/// written, escapes and cut mark included, cut at a character boundary and
/// marked `...` when cut, with control, invisible and reordering characters
/// written as `\u{..}` escapes, so a quote can't forge a log line, recolor a
/// terminal or reorder its line.
pub fn quote(text: &str) -> String {
    let mut quoted = String::new();
    let mut written = String::new();
    // The longest prefix that leaves room for the cut mark.
    let mut room = 0;
    for c in text.chars() {
        written.clear();
        if is_hidden(c) {
            let _ = write!(written, "\\u{{{:04x}}}", u32::from(c));
        } else {
            written.push(c);
        }
        if quoted.len() + written.len() > MAX_QUOTE_BYTES {
            quoted.truncate(room);
            quoted.push_str(CUT);
            break;
        }
        quoted.push_str(&written);
        if quoted.len() + CUT.len() <= MAX_QUOTE_BYTES {
            room = quoted.len();
        }
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

fn check_resource_id(id: &str) -> Result<(), NameError> {
    if id.is_empty() {
        Err(NameError::EmptyResource)
    } else if id.len() > MAX_RESOURCE_ID_BYTES {
        Err(NameError::ResourceIdTooLong(quote(id)))
    } else {
        check_printable(id)
    }
}

/// `namespace:id`
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ResourceName {
    namespace: NamespaceName,
    id: ResourceId,
}

impl ResourceName {
    /// The resource `id` under `namespace`. Each part guards its own rules,
    /// so the pair has none of its own.
    pub fn new(namespace: NamespaceName, id: ResourceId) -> Self {
        Self { namespace, id }
    }

    pub fn namespace(&self) -> &NamespaceName {
        &self.namespace
    }

    pub fn id(&self) -> &ResourceId {
        &self.id
    }
}

impl FromStr for ResourceName {
    type Err = NameError;

    fn from_str(text: &str) -> Result<Self, NameError> {
        let Some((namespace, id)) = text.split_once(':') else {
            return Err(NameError::Resource(quote(text)));
        };
        if namespace.is_empty() || id.is_empty() {
            return Err(NameError::Resource(quote(text)));
        }
        Ok(Self::new(namespace.parse()?, id.parse()?))
    }
}

impl fmt::Display for ResourceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.namespace, self.id)
    }
}

/// `namespace:id#relation`
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SubjectsetName {
    resource: ResourceName,
    relation: RelationName,
}

impl SubjectsetName {
    pub fn new(resource: ResourceName, relation: RelationName) -> Self {
        Self { resource, relation }
    }

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

/// A subject as a caller names it.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum SubjectName {
    Identity(Identity),
    /// `namespace:id`, a resource member.
    Resource(ResourceName),
    Subjectset(SubjectsetName),
}

impl fmt::Display for SubjectName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Identity(identity) => identity.fmt(f),
            Self::Resource(resource) => resource.fmt(f),
            Self::Subjectset(set) => set.fmt(f),
        }
    }
}

/// A fact as a caller names it: `namespace:id#relation@subject`.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct FactName {
    pub subjectset: SubjectsetName,
    pub subject: SubjectName,
}

impl fmt::Display for FactName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.subjectset, self.subject)
    }
}
