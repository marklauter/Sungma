//! Runs the TOML namespace document parser on arbitrary bytes: it must never
//! panic, overflow its stack or abort, and a namespace it accepts must print,
//! in either form, as a document that parses back equal.

#![no_main]

use libfuzzer_sys::fuzz_target;
use sungma_lang::namespace;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(parsed) = namespace::parse_toml(text) else {
        return;
    };
    let printed = namespace::print_toml(&parsed.namespace, &parsed.relations)
        .expect("a parsed namespace prints");
    let again = namespace::parse_toml(&printed).expect("a printed namespace parses");
    let json = namespace::print(&parsed.namespace, &parsed.relations)
        .expect("a parsed namespace prints as JSON");
    let from_json = namespace::parse(&json).expect("a printed JSON namespace parses");
    let sorted = |document: &namespace::NamespaceDocument| {
        let mut relations: Vec<_> = document.relations.relations().collect();
        relations.sort_by_key(|(name, _)| *name);
        relations
            .into_iter()
            .map(|(name, rewrite)| (name.clone(), rewrite.clone()))
            .collect::<Vec<_>>()
    };
    let printed_again = namespace::print_toml(&again.namespace, &again.relations)
        .expect("a reparsed namespace prints");
    assert_eq!(printed_again, printed);
    assert_eq!(again.namespace, parsed.namespace);
    assert_eq!(sorted(&again), sorted(&parsed));
    assert_eq!(from_json.namespace, parsed.namespace);
    assert_eq!(sorted(&from_json), sorted(&parsed));
});
