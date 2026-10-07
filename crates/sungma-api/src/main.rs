//! `sungma-api <facts.json> <theory.json>...` serves Check on `SUNGMA_ADDR`,
//! `127.0.0.1:8080` by default.

use std::{env, error::Error, fs, sync::Arc};

use sungma_api::{AppState, router};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    // At least one theory: with none, every check would be denied.
    let (facts, theories) = match args.as_slice() {
        [facts, theories @ ..] if !theories.is_empty() => (facts, theories),
        _ => return Err("usage: sungma-api <facts.json> <theory.json>...".into()),
    };
    let theories = theories
        .iter()
        .map(fs::read_to_string)
        .collect::<Result<Vec<_>, _>>()?;
    let theories: Vec<&str> = theories.iter().map(String::as_str).collect();
    let state = AppState::load(&theories, &fs::read_to_string(facts)?)?;
    let addr = env::var("SUNGMA_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_owned());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("sungma-api listening on {addr}");
    axum::serve(listener, router(Arc::new(state))).await?;
    Ok(())
}
