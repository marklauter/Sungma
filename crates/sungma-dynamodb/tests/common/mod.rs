//! A table of its own for each test, on DynamoDB Local at
//! `SUNGMA_DYNAMODB_URL`, `http://localhost:8000` by default.
//! `scripts/dynamodb.sh` starts it.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aws_sdk_dynamodb::Client;
use sungma_dynamodb::{Table, local, testkit};
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

/// A new table of the test's own, deleted once the test is done with it.
pub async fn table() -> Table {
    static SWEPT: OnceCell<()> = OnceCell::const_new();
    let endpoint =
        std::env::var("SUNGMA_DYNAMODB_URL").unwrap_or_else(|_| "http://localhost:8000".to_owned());
    let client = local(&endpoint);
    SWEPT.get_or_init(|| sweep(&client)).await;
    testkit::table(&endpoint, "sungma-dynamodb")
        .await
        .unwrap_or_else(|error| {
            panic!("DynamoDB Local at {endpoint} refused a table; run scripts/dynamodb.sh: {error}")
        })
}
