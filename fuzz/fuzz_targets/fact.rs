//! Runs the fact document parser on arbitrary bytes: it must never panic,
//! overflow its stack or abort, and a document it refuses must declare no
//! facts.

#![no_main]

use std::{
    pin::pin,
    task::{Context, Poll, Waker},
};

use libfuzzer_sys::fuzz_target;
use sungma::{
    memory::{MemoryDictionary, MemoryFactStore},
    store::FactStore,
};
use sungma_lang::fact;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let dictionary = MemoryDictionary::default();
    let facts = MemoryFactStore::default();
    if fact::load_facts(text, &dictionary, &facts).is_err() {
        // The in-memory store answers at once, so one poll settles it.
        let Poll::Ready(head) = pin!(facts.head()).poll(&mut Context::from_waker(Waker::noop()))
        else {
            panic!("the in-memory store answered later");
        };
        assert_eq!(head.expect("the in-memory store never fails").0, 0);
    }
});
