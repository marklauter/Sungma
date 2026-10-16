//! Sungma's shared kernel: the primitives every context builds on. Names
//! and the rules they keep, ids and revisions, the fact graph, namespaces
//! and their rewrite trees, and namespace versions with the epochs that
//! keep orphaned facts dead.
//!
//! The kernel works on interned integer ids. The names context resolves
//! names to ids at the edge.

pub mod graph;
pub mod id;
pub mod name;
pub mod namespace;
pub mod non_empty;
pub mod revision;
pub mod rewrite;
pub mod store;
pub mod version;
