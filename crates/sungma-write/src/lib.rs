//! The write side: check a write against the rules that span models, then
//! apply it through the store's writer ports. [`service`] takes writes as
//! a caller names them.
//!
//! The `http` feature adds the Write service's routes, in `http`, and the
//! `server` feature its binary, `sungma-write`.

pub mod fact;
#[cfg(feature = "http")]
pub mod http;
pub mod rows;
pub mod service;
pub mod store;

#[cfg(feature = "testkit")]
pub mod testkit;
