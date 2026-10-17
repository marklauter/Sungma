//! One DynamoDB table as a key-value table: items under binary hash and range
//! keys, units of work as transactions, and sequences as atomic adds. It
//! knows nothing of what the items mean; each context lays out its own.
//! Development runs it against DynamoDB Local. `docs/specs/dynamodb-adapter.md`
//! specifies it.

pub mod items;
mod table;

use aws_sdk_dynamodb::{
    Client, Config,
    config::{BehaviorVersion, Credentials, Region},
};

pub use table::Table;

/// A client for DynamoDB Local at `endpoint`, such as
/// `http://localhost:8000`. Local takes any credentials.
pub fn local(endpoint: &str) -> Client {
    let config = Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .endpoint_url(endpoint)
        .region(Region::new("us-east-1"))
        .credentials_provider(Credentials::new("local", "local", None, None, "sungma"))
        .build();
    Client::from_conf(config)
}
