//! Runs the namespace document parser on arbitrary bytes: it must never panic,
//! overflow its stack or abort, and a namespace it accepts must print as a
//! document that parses back equal.

#![no_main]

use libfuzzer_sys::fuzz_target;
use sungma_lang::namespace;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(parsed) = namespace::parse(text) else {
        return;
    };
    let printed = namespace::print(&parsed.namespace, &parsed.relations);
    let again = namespace::parse(&printed).expect("a printed namespace parses");
    let sorted = |document: &namespace::NamespaceDocument| {
        let mut relations: Vec<_> = document.relations.relations().collect();
        relations.sort_by_key(|(name, _)| *name);
        relations
            .into_iter()
            .map(|(name, rewrite)| (name.clone(), rewrite.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(again.namespace, parsed.namespace);
    assert_eq!(sorted(&again), sorted(&parsed));
});
