//! The `sungma-api` binary's arguments and stores. Each run is given an
//! address it can't bind, so none starts a server.

use std::process::{Command, Output};

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
    let mut command = Command::new(env!("CARGO_BIN_EXE_sungma-api"));
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
    assert!(stderr(&output).contains("usage: sungma-api"));
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

/// Runs on DynamoDB Local, creating the table, and seeds it; a second
/// run finds the table and serves it without seeding it again.
#[test]
fn the_server_runs_on_dynamodb_when_given_a_url() {
    let url =
        std::env::var("SUNGMA_DYNAMODB_URL").unwrap_or_else(|_| "http://localhost:8000".to_owned());
    let table = format!("sungma-api-{}", std::process::id());
    let env = [
        ("SUNGMA_DYNAMODB_URL", url.as_str()),
        ("SUNGMA_DYNAMODB_TABLE", table.as_str()),
    ];
    let facts = format!("{FIXTURES}/docs.facts.json");
    let seeded = run(&seed(&facts), &env);
    assert!(stderr(&seeded).contains(UNBOUND), "{}", stderr(&seeded));
    let again = run(&seed(&facts), &env);
    let stderr = stderr(&again);
    assert!(stderr.contains(UNBOUND), "{stderr}");
    let stdout = String::from_utf8_lossy(&again.stdout);
    assert!(stdout.contains("isn't seeded"), "{stdout}");
}
