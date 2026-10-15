//! Runs the fact document parser on arbitrary bytes, against the fixture
//! namespaces: it must never panic, overflow its stack or abort, and a
//! document it refuses must store no fact.

#![no_main]

use std::{
    pin::pin,
    task::{Context, Poll, Waker},
};

use libfuzzer_sys::fuzz_target;
use sungma::store::FactReader;
use sungma_lang::{fact, namespace::load_namespaces};
use sungma_memory::{MemoryDictionary, MemoryStore};

const NAMESPACES: [&str; 3] = [
    include_str!("../../crates/sungma/tests/fixtures/namespaces/file.json"),
    include_str!("../../crates/sungma/tests/fixtures/namespaces/folder.json"),
    include_str!("../../crates/sungma/tests/fixtures/namespaces/group.json"),
];

fn head(facts: &MemoryStore) -> u64 {
    // The in-memory store answers at once, so one poll settles it.
    let Poll::Ready(head) = pin!(facts.head()).poll(&mut Context::from_waker(Waker::noop())) else {
        panic!("the in-memory store answered later");
    };
    head.expect("the in-memory store never fails").0
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let dictionary = MemoryDictionary::default();
    let facts = MemoryStore::default();
    load_namespaces(&NAMESPACES, &dictionary, &facts).expect("the fixtures load");
    let declared = head(&facts);
    if fact::load_facts(text, &dictionary, &facts).is_err() {
        assert_eq!(head(&facts), declared);
    }
});
