//! A table of its own for each test, on DynamoDB Local at
//! `SUNGMA_DYNAMODB_URL`, `http://localhost:8000` by default.
//! `scripts/dynamodb.sh` starts it.

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use aws_sdk_dynamodb::Client;
use sungma_dynamodb::{Table, local};
use tokio::sync::OnceCell;

/// Tables older than this, from earlier runs, are deleted. Younger ones
/// may belong to a run still going.
const KEPT: Duration = Duration::from_secs(600);

/// Deletes the tests' tables that earlier runs left, so a long-running
/// DynamoDB Local doesn't fill up.
async fn sweep(client: &Client) {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let mut start = None;
    loop {
        let page = client
            .list_tables()
            .set_exclusive_start_table_name(start)
            .send()
            .await
            .unwrap();
        for name in page.table_names() {
            if !name.starts_with("sungma-") {
                continue;
            }
            let described = client.describe_table().table_name(name).send().await;
            let created = described
                .ok()
                .and_then(|output| output.table?.creation_date_time)
                .map(|created| Duration::from_secs(created.secs().max(0) as u64));
            if created.is_some_and(|created| now.saturating_sub(created) > KEPT) {
                let _ = client.delete_table().table_name(name).send().await;
            }
        }
        start = page.last_evaluated_table_name;
        if start.is_none() {
            return;
        }
    }
}

/// A new table, named for this process and a counter so parallel tests
/// and runs don't share one.
pub async fn table() -> Table {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    static SWEPT: OnceCell<()> = OnceCell::const_new();
    let endpoint =
        std::env::var("SUNGMA_DYNAMODB_URL").unwrap_or_else(|_| "http://localhost:8000".to_owned());
    let client = local(&endpoint);
    SWEPT.get_or_init(|| sweep(&client)).await;
    let name = format!(
        "sungma-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let table = Table::new(client, name);
    table.create().await.unwrap_or_else(|error| {
        panic!("DynamoDB Local at {endpoint} refused a table; run scripts/dynamodb.sh: {error}")
    });
    table
}
