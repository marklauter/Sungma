//! The Write API: check a write against the rules that span models, then
//! apply it through the store's writer ports.
//!
//! A theory write comes first; fact writes join it with the Write service.

pub mod intern;
pub mod store;
