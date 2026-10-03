//! Sungma core: the fact model, rewrite trees, and the Check evaluator.
//!
//! Everything in the core works on interned integer ids. Strings are
//! resolved to ids at the edge, through a [`store::Dictionary`].

pub mod check;
pub mod fixture;
pub mod memory;
pub mod model;
pub mod resolve;
pub mod rewrite;
pub mod store;
pub mod theory;
