//! The write side: check a write against the rules that span models, then
//! apply it through the store's writer ports. [`service`] takes writes as
//! a caller names them.

pub mod fact;
pub mod intern;
pub mod rows;
pub mod service;
pub mod store;

#[cfg(feature = "testkit")]
pub mod testkit;
