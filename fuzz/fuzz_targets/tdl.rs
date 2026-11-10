//! Runs the TDL parser on arbitrary bytes: it must never panic, overflow its
//! stack or abort, and the namespaces it accepts must print as a document
//! that parses back equal, and print the same again.

#![no_main]

use libfuzzer_sys::fuzz_target;
use sungma_lang::tdl::{self, NamespaceDocument, PrintError};

fn print(documents: &[NamespaceDocument]) -> Result<String, PrintError> {
    let namespaces: Vec<_> = documents
        .iter()
        .map(|document| (&document.namespace, &document.relations))
        .collect();
    tdl::print(&namespaces)
}

/// Each namespace's relations, with their constraints, sorted, so documents
/// compare by what they declare.
fn declared(documents: &[NamespaceDocument]) -> Vec<String> {
    let mut declared: Vec<_> = documents
        .iter()
        .flat_map(|document| {
            document.relations.relations().map(|(name, rewrite)| {
                let mut constraints = document.relations.constraints(name).to_vec();
                constraints.sort();
                format!("{} {name} {rewrite:?} {constraints:?}", document.namespace)
            })
        })
        .collect();
    declared.extend(
        documents
            .iter()
            .map(|document| document.namespace.to_string()),
    );
    declared.sort();
    declared
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(parsed) = tdl::parse(text) else {
        return;
    };
    // Spaced out, a document near its limit can print past it, and a
    // namespace in a theory can name one without a dot, which no document
    // in a theory can write.
    let printed = match print(&parsed) {
        Ok(printed) => printed,
        Err(PrintError::TooLarge | PrintError::Undotted { .. }) => return,
        Err(error) => panic!("parsed namespaces print: {error}"),
    };
    let again = tdl::parse(&printed).expect("printed namespaces parse");
    assert_eq!(declared(&again), declared(&parsed));
    assert_eq!(print(&again), Ok(printed));
});
