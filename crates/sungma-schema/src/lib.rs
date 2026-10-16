//! The rows contexts share on one table: where each kind of row lives and
//! what it holds. The write context writes them, and the read contexts read
//! them back. Names have rows of their own, in `sungma-names`. Each module
//! keeps its partitions under a tag of its own, so no two kinds of row meet.
//!
//! A row Sungma didn't write is refused with a store error that says what
//! was wrong with it.

pub mod facts;
pub mod head;
pub mod namespaces;

use sungma::store::StoreError;
use sungma_edge::EdgeError;

/// The partition tags. Names use 3.
const FACT: u8 = 1;
const NAMESPACE: u8 = 2;
const HEAD: u8 = 4;

pub fn failed(error: EdgeError) -> StoreError {
    StoreError(error.0)
}
