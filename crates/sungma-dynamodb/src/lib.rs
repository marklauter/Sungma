//! The storage ports on one DynamoDB table: facts, namespace versions,
//! names and the head, on one timeline. Development runs it against
//! DynamoDB Local. `docs/specs/dynamodb-adapter.md` specifies it.
//!
//! A [`DynamoReader`] serves the read ports and a [`DynamoWriter`] the
//! write ports, so code that only reads is given only a reader. Both share
//! one [`Table`].

pub mod edge;
mod facts;
pub mod key;
mod names;
mod namespaces;
mod reader;
mod table;
mod writer;

use aws_sdk_dynamodb::{
    Client, Config,
    config::{BehaviorVersion, Credentials, Region},
};

pub use reader::DynamoReader;
pub use table::Table;
pub use writer::DynamoWriter;

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
