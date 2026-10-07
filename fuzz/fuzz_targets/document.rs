//! Runs the theory document parser on arbitrary bytes: it must never panic,
//! overflow its stack or abort, and a theory it accepts must print as a
//! document that parses back equal.

#![no_main]

use libfuzzer_sys::fuzz_target;
use sungma_core::document;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(parsed) = document::parse(text) else {
        return;
    };
    let printed = document::print(&parsed.theory, &parsed.relations);
    let again = document::parse(&printed).expect("a printed theory parses");
    let sorted = |mut relations: Vec<_>| {
        relations.sort_by(|(a, _): &(sungma_core::name::RelationName, _), (b, _)| a.cmp(b));
        relations
    };
    assert_eq!(again.theory, parsed.theory);
    assert_eq!(sorted(again.relations), sorted(parsed.relations));
});
