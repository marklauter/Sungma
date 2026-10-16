//! Where names live: a partition per pool, sorted by name and by id.

use sungma::store::StoreError;

use crate::store::Pool;
use sungma_edge::{EdgeError, key::Key};

/// The partition tag for names, apart from every other context's.
const NAMES: u8 = 3;

/// Sort key tags inside a pool's partition.
const COUNTER: u8 = 0;
const NAME: u8 = 1;
const ID: u8 = 2;

/// The attribute a name row keeps its id in, and an id row its name.
pub const ID_ATTRIBUTE: &str = "id";
pub const NAME_ATTRIBUTE: &str = "name";

fn pool(pool: Pool) -> (u8, u8) {
    let pool = match pool {
        Pool::Namespaces => 0,
        Pool::Relations => 1,
    };
    (NAMES, pool)
}

/// The row that maps `name` to its id.
pub fn name(at: Pool, name: &str) -> Key {
    Key::new(pool(at), (NAME, name))
}

/// The row that maps `id` to its name.
pub fn id(at: Pool, id: u32) -> Key {
    Key::new(pool(at), (ID, id))
}

/// The pool's id counter.
pub fn counter(at: Pool) -> Key {
    Key::new(pool(at), COUNTER)
}

pub fn failed(error: EdgeError) -> StoreError {
    StoreError(error.0)
}
