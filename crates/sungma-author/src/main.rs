//! `sungma-author` serves Author, with its sessions in memory. Its
//! environment is in [`Settings`].

use std::{env, error::Error, sync::Arc};

use sungma_author::Settings;
use sungma_bff::Bff;
use sungma_clock::OsTime;
use tower_sessions::MemoryStore;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let settings =
        Settings::from(|name| env::var(name).ok()).map_err(|missing| missing.to_string())?;
    let bff = Bff::discover(settings.bff, Arc::new(OsTime)).await?;
    let app = sungma_author::app(&bff, settings.provider, MemoryStore::default());
    let listener = tokio::net::TcpListener::bind(&settings.addr).await?;
    println!("sungma-author listening on {}", settings.addr);
    axum::serve(listener, app).await?;
    Ok(())
}
