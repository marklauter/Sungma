//! The `sungma-check` binary's arguments and stores. Each run is given an
//! address it can't bind, so none starts a server.

use std::process::{Command, Output};

use sungma_dynamodb::{Table, local};

/// What a run that got as far as binding reports.
const UNBOUND: &str = "invalid socket address";

fn run(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sungma-check"));
    command
        .args(args)
        .env("SUNGMA_ADDR", "not an address")
        .env_remove("SUNGMA_DYNAMODB_URL");
    command.envs(env.iter().copied()).output().unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// `SUNGMA_DYNAMODB_URL`, or none when it's unset or empty. The tests that
/// need DynamoDB Local pass without running when there's none.
fn dynamodb_url() -> Option<String> {
    std::env::var("SUNGMA_DYNAMODB_URL")
        .ok()
        .filter(|url| !url.is_empty())
}

#[test]
fn the_server_takes_no_arguments() {
    let output = run(&["facts.json"], &[]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("usage: sungma-check"));
}

#[test]
fn the_server_starts_in_memory() {
    let output = run(&[], &[]);
    assert!(!output.status.success());
    let stderr = stderr(&output);
    assert!(stderr.contains(UNBOUND), "{stderr}");
}

/// Refuses a table that doesn't exist, and serves one that does.
#[tokio::test]
async fn the_server_runs_on_dynamodb_when_given_a_url() {
    let Some(url) = dynamodb_url() else { return };
    let name = format!("sungma-check-{}", std::process::id());
    let env = [
        ("SUNGMA_DYNAMODB_URL", url.as_str()),
        ("SUNGMA_DYNAMODB_TABLE", name.as_str()),
    ];
    let missing = run(&[], &env);
    assert!(!missing.status.success());
    let stderr = stderr(&missing);
    assert!(stderr.contains("doesn't exist"), "{stderr}");
    assert!(!stderr.contains(UNBOUND), "{stderr}");

    Table::new(local(&url), name.clone())
        .create()
        .await
        .unwrap_or_else(|error| {
            panic!("DynamoDB Local at {url} refused a table; run scripts/dynamodb.sh: {error}")
        });
    let output = run(&[], &env);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(UNBOUND), "{stderr}");
}
