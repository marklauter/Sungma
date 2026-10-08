//! The `sungma-api` binary's arguments. Each run is given an address it
//! can't bind, so none starts a server.

use std::process::{Command, Output};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../sungma/tests/fixtures");

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sungma-api"))
        .args(args)
        .env("SUNGMA_ADDR", "not an address")
        .output()
        .unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn the_server_needs_facts_and_at_least_one_theory() {
    let facts = format!("{FIXTURES}/docs.facts.json");
    for args in [vec![], vec![facts.as_str()]] {
        let output = run(&args);
        assert!(!output.status.success(), "{args:?}");
        assert!(stderr(&output).contains("usage: sungma-api"), "{args:?}");
    }
}

#[test]
fn the_server_loads_its_files_before_it_binds() {
    let facts = format!("{FIXTURES}/docs.facts.json");
    let theory = format!("{FIXTURES}/theories/file.json");
    let output = run(&[&facts, &theory]);
    assert!(!output.status.success());
    let stderr = stderr(&output);
    assert!(!stderr.contains("usage"), "{stderr}");
}
