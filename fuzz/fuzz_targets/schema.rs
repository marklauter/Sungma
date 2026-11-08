//! Runs the Sungma schema language parser on arbitrary bytes: it must never
//! panic, overflow its stack or abort, and the namespaces it accepts must
//! print as a document that parses back equal, and print the same again.

#![no_main]

use libfuzzer_sys::fuzz_target;
use sungma_lang::{
    namespace::{NamespaceDocument, PrintError},
    schema,
};

fn print(documents: &[NamespaceDocument]) -> Result<String, PrintError> {
    let namespaces: Vec<_> = documents
        .iter()
        .map(|document| (&document.namespace, &document.relations))
        .collect();
    schema::print(None, &namespaces)
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(parsed) = schema::parse(text) else {
        return;
    };
    // Spaced out, a document near the size limit can print past it.
    let printed = match print(&parsed) {
        Ok(printed) => printed,
        Err(PrintError::TooLarge) => return,
        Err(error) => panic!("parsed namespaces print: {error}"),
    };
    let again = schema::parse(&printed).expect("printed namespaces parse");
    assert_eq!(print(&again), Ok(printed));
});
