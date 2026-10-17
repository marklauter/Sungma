//! The memory table and sequence pass every table test.

use sungma_table::{
    memory::{MemorySequence, MemoryTable},
    table_tests,
};

table_tests!(MemoryTable::new(), MemorySequence::new());
