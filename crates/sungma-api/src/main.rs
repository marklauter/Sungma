//! `sungma-api <theories.json> <facts.json>` serves Check on `SUNGMA_ADDR`,
//! `127.0.0.1:8080` by default.

use std::{env, error::Error, fs, sync::Arc};

use sungma_api::{AppState, router};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    let [theories, facts] = args.as_slice() else {
        return Err("usage: sungma-api <theories.json> <facts.json>".into());
    };
    let state = AppState::load(&fs::read_to_string(theories)?, &fs::read_to_string(facts)?)?;
    let addr = env::var("SUNGMA_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_owned());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("sungma-api listening on {addr}");
    axum::serve(listener, router(Arc::new(state))).await?;
    Ok(())
}
