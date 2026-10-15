//! The Write API: check a write against the rules that span models, then
//! apply it through the store's writer ports.
//!
//! A namespace write comes first; fact writes join it with the Write service.

pub mod fact;
pub mod intern;
pub mod store;

#[cfg(feature = "testkit")]
pub mod testkit;
