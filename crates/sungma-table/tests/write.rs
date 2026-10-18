//! Units of work, and writers behind a reference or an `Arc`.

use std::sync::Arc;

use sungma_table::{
    item::Item,
    key::Key,
    memory::{MAX_STEPS, MemoryTable},
    write::{ItemWriter, Step, Work},
};

#[test]
fn a_step_pushed_is_in_the_work() {
    let mut work = Work::new();
    assert!(work.is_empty());
    let step = Step::Insert {
        key: Key::new(1u8, 2u8),
        item: Item::new(),
    };
    work.push(step.clone());
    assert!(!work.is_empty());
    assert_eq!(work, Work(vec![step]));
}

#[test]
fn a_shared_writer_takes_as_many_steps_as_its_table() {
    let table = MemoryTable::new();
    assert_eq!(ItemWriter::max_steps(&&table), MAX_STEPS);
    assert_eq!(Arc::new(table).max_steps(), MAX_STEPS);
}
