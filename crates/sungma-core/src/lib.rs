//! Sungma core: the fact model, rewrite trees, the closure of a subjectset,
//! which answers Contains and Expand, and the decisions Check records for
//! audit and replay.
//!
//! Everything in the core works on interned integer ids. Strings are
//! resolved to ids at the edge, through a [`store::Dictionary`].

pub mod check;
pub mod closure;
pub mod decision;
pub mod document;
pub mod fixture;
pub mod intern;
pub mod memory;
pub mod model;
pub mod name;
pub mod replay;
pub mod resolve;
pub mod rewrite;
pub mod store;
pub mod theory;
