//! The `sungma-author` binary's settings. Each run is given an address it
//! can't bind, so none starts a server.

use std::{process::Output, sync::Arc};

use sungma_bff::testkit::{CLIENT_ID, CLIENT_SECRET, FakeIdp};
use sungma_clock::OsTime;

/// What a run that got as far as binding reports.
const UNBOUND: &str = "invalid socket address";

async fn run(env: &[(&str, &str)]) -> Output {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_sungma-author"));
    for name in [
        "SUNGMA_AUTHOR_ISSUER",
        "SUNGMA_AUTHOR_CLIENT_ID",
        "SUNGMA_AUTHOR_CLIENT_SECRET",
        "SUNGMA_AUTHOR_ORIGIN",
        "SUNGMA_AUTHOR_PROVIDER",
    ] {
        command.env_remove(name);
    }
    command.env("SUNGMA_AUTHOR_ADDR", "not an address");
    command.envs(env.iter().copied()).output().await.unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[tokio::test]
async fn the_server_needs_its_identity_provider() {
    let output = run(&[]).await;
    assert!(!output.status.success());
    assert!(stderr(&output).contains("SUNGMA_AUTHOR_ISSUER isn't set"));
}

#[tokio::test]
async fn the_server_discovers_its_identity_provider_before_it_binds() {
    let idp = FakeIdp::start(Arc::new(OsTime)).await;
    let client = [
        ("SUNGMA_AUTHOR_CLIENT_ID", CLIENT_ID),
        ("SUNGMA_AUTHOR_CLIENT_SECRET", CLIENT_SECRET),
    ];
    let mut found = client.to_vec();
    found.push(("SUNGMA_AUTHOR_ISSUER", idp.issuer()));
    let output = run(&found).await;
    let stderr = stderr(&output);
    assert!(stderr.contains(UNBOUND), "{stderr}");

    let mut lost = client.to_vec();
    lost.push(("SUNGMA_AUTHOR_ISSUER", "http://127.0.0.1:1"));
    let output = run(&lost).await;
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains(UNBOUND), "{stderr}");
}
