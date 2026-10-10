//! Sungma core: the model every service shares. Names, ids and revisions,
//! the fact graph, theories and their rewrite trees, the storage ports, the
//! closure of a subjectset, which answers Contains and Expand, and the
//! decisions Check records for audit and replay.
//!
//! Everything in the core works on interned integer ids. Strings are
//! resolved to ids at the edge, through a [`store::Dictionary`].

pub mod closure;
pub mod decision;
pub mod graph;
pub mod id;
pub mod intern;
pub mod name;
pub mod replay;
pub mod revision;
pub mod rewrite;
pub mod store;
pub mod theory;
