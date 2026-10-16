//! Interned names over the store edge: a row per name holding its id, a
//! row per id holding its name, and a counter per pool that leases ids.
//! [`read::NameRows`] reads names and [`write::NameRowWriter`] stores
//! them; the interner mints through both.

pub mod read;
mod rows;
pub mod write;
