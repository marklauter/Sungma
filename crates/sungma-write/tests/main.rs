//! The `sungma-write` binary's arguments and stores. Each run is given an
//! address it can't bind, so none starts a server.

mod common;

use std::{
    fs,
    process::{Command, Output},
};

use serde_json::Value;
use sungma_dynamodb::{Table, items::DynamoTable, local};
use sungma_read::namespaces::NamespaceReadService;

use common::dynamodb_url;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../sungma/tests/fixtures");

/// What a run that got as far as binding reports.
const UNBOUND: &str = "invalid socket address";

const NAMESPACES: [&str; 3] = [
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../sungma/tests/fixtures/namespaces/file.json"
    ),
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../sungma/tests/fixtures/namespaces/folder.json"
    ),
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../sungma/tests/fixtures/namespaces/group.json"
    ),
];

fn run(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sungma-write"));
    command
        .args(args)
        .env("SUNGMA_ADDR", "not an address")
        .env_remove("SUNGMA_DYNAMODB_URL");
    command.envs(env.iter().copied()).output().unwrap()
}

/// The facts file and the docs namespaces, as arguments.
fn seed(facts: &str) -> Vec<&str> {
    let mut args = vec![facts];
    args.extend(NAMESPACES);
    args
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn facts_need_at_least_one_namespace() {
    let facts = format!("{FIXTURES}/docs.facts.json");
    let output = run(&[&facts], &[]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("usage: sungma-write"));
}

#[test]
fn the_server_starts_without_seed_data() {
    let output = run(&[], &[]);
    assert!(!output.status.success());
    let stderr = stderr(&output);
    assert!(stderr.contains(UNBOUND), "{stderr}");
}

#[test]
fn the_server_seeds_its_files_before_it_binds() {
    let facts = format!("{FIXTURES}/docs.facts.json");
    let output = run(&seed(&facts), &[]);
    assert!(!output.status.success());
    let stderr = stderr(&output);
    assert!(!stderr.contains("usage"), "{stderr}");
    assert!(stderr.contains(UNBOUND), "{stderr}");
}

#[test]
fn a_seed_that_is_refused_stops_the_server() {
    let facts = format!("{FIXTURES}/docs.facts.json");
    let missing = format!("{FIXTURES}/namespaces/missing.json");
    let output = run(&[&facts, &missing], &[]);
    assert!(!output.status.success());
    assert!(!stderr(&output).contains(UNBOUND));
}

#[test]
fn a_seed_that_breaks_the_grammar_stops_the_server() {
    let facts = format!("{FIXTURES}/docs.facts.json");
    let refused = format!("{}/refused.json", env!("CARGO_TARGET_TMPDIR"));
    fs::write(&refused, r#"{ "file": { "viewer": "editor" } }"#).unwrap();
    // A namespace document, then a fact document, that's refused.
    for args in [[facts.as_str(), &refused], [&refused, NAMESPACES[0]]] {
        let output = run(&args, &[]);
        assert!(!output.status.success());
        let stderr = stderr(&output);
        assert!(!stderr.contains(UNBOUND), "{stderr}");
    }
}

/// Runs on DynamoDB Local, creating the table, and seeds it with the
/// printed documents; a second run finds the table and serves it without
/// seeding it again.
#[tokio::test]
async fn the_server_runs_on_dynamodb_when_given_a_url() {
    let Some(url) = dynamodb_url() else { return };
    let name = format!("sungma-write-{}", std::process::id());
    let env = [
        ("SUNGMA_DYNAMODB_URL", url.as_str()),
        ("SUNGMA_DYNAMODB_TABLE", name.as_str()),
    ];
    let facts = format!("{FIXTURES}/docs.facts.json");
    let seeded = run(&seed(&facts), &env);
    assert!(stderr(&seeded).contains(UNBOUND), "{}", stderr(&seeded));

    let table = DynamoTable::new(Table::new(local(&url), name.clone()));
    let names = sungma_node::names(&table);
    let read = NamespaceReadService::new(&names, &table)
        .read(&"file".parse().unwrap(), None)
        .await
        .unwrap();
    let stored: Value = serde_json::from_str(&read.version.unwrap().document).unwrap();
    let file: Value = serde_json::from_str(&fs::read_to_string(NAMESPACES[0]).unwrap()).unwrap();
    assert_eq!(stored, file);

    let again = run(&seed(&facts), &env);
    let stderr = stderr(&again);
    assert!(stderr.contains(UNBOUND), "{stderr}");
    let stdout = String::from_utf8_lossy(&again.stdout);
    assert!(stdout.contains("isn't seeded"), "{stdout}");
}
