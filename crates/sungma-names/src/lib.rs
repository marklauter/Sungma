//! The names context: names and the ids interned for them. [`store`] holds
//! the name ports; [`read::NameRows`] reads names from their rows on the
//! store edge and [`write::NameRowWriter`] stores them, a row per name
//! holding its id, a row per id holding its name, and a counter per pool
//! that leases ids. [`intern::LeasingInterner`] mints through both, and
//! [`resolve`] maps a caller's names to ids and back.

pub mod intern;
pub mod read;
pub mod resolve;
mod rows;
pub mod store;
pub mod write;
