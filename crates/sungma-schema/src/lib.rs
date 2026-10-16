//! The rows contexts share on one table: where each kind of row lives and
//! what it holds. The write context writes them, and the read contexts read
//! them back. Names have rows of their own, in `sungma-names`. Each kind of
//! row keeps its partitions under a tag of its own, from [`tags`], so no
//! two kinds of row meet.
//!
//! A row Sungma didn't write is refused with a store error that says what
//! was wrong with it.

pub mod facts;
pub mod head;
pub mod namespaces;

use sungma::store::StoreError;
use sungma_edge::EdgeError;

/// Every partition's first byte, one per kind of row, so no two kinds
/// share a partition. Every context's rows take their tag from here.
pub mod tags {
    pub const FACT: u8 = 1;
    pub const NAMESPACE: u8 = 2;
    pub const NAMES: u8 = 3;
    pub const HEAD: u8 = 4;

    /// Every tag, for the test that they're distinct.
    pub const ALL: [u8; 4] = [FACT, NAMESPACE, NAMES, HEAD];
}

pub fn failed(error: EdgeError) -> StoreError {
    StoreError(error.0)
}
