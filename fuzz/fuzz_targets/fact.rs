//! Runs the fact document parser on arbitrary bytes, against the fixture
//! namespaces on a memory node: it must never panic, overflow its stack or
//! abort, and a document it refuses must store no fact.

#![no_main]

use std::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};

use libfuzzer_sys::fuzz_target;
use sungma_node::testkit::World;

const NAMESPACES: [&str; 3] = [
    include_str!("../../crates/sungma/tests/fixtures/namespaces/file.json"),
    include_str!("../../crates/sungma/tests/fixtures/namespaces/folder.json"),
    include_str!("../../crates/sungma/tests/fixtures/namespaces/group.json"),
];

/// Runs `future` to its end. The memory node never waits on anything, so
/// each poll makes progress.
fn run<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let world = run(World::load(&NAMESPACES, "[]")).expect("the fixtures load");
    let declared = run(world.head());
    if run(world.load_facts(text)).is_err() {
        assert_eq!(run(world.head()), declared);
    }
});
