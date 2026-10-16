//! The memory table and sequence pass every edge test.

use sungma_edge::{
    edge_tests,
    memory::{MemorySequence, MemoryTable},
};

edge_tests!(MemoryTable::new(), MemorySequence::new());
