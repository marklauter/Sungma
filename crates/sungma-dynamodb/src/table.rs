//! The table: binary hash and range keys, and nothing else declared, since
//! every other attribute is the item's own.

use std::sync::Arc;

use aws_sdk_dynamodb::{
    Client,
    error::DisplayErrorContext,
    types::{AttributeDefinition, BillingMode, KeySchemaElement, KeyType, ScalarAttributeType},
};
use sungma_edge::EdgeError;

/// The hash key and range key attribute names, the edge's key attributes.
pub const PK: &str = sungma_edge::item::PARTITION_KEY;
pub const SK: &str = sungma_edge::item::SORT_KEY;

fn failed(error: impl std::error::Error) -> EdgeError {
    EdgeError(DisplayErrorContext(error).to_string())
}

/// A table and the client that reaches it. Cloning shares the client.
#[derive(Clone, Debug)]
pub struct Table {
    client: Client,
    name: Arc<str>,
}

impl Table {
    pub fn new(client: Client, name: impl Into<Arc<str>>) -> Self {
        Self {
            client,
            name: name.into(),
        }
    }

    /// Creates the table: binary hash and range keys, billed per request.
    pub async fn create(&self) -> Result<(), EdgeError> {
        let attribute = |name: &str| {
            AttributeDefinition::builder()
                .attribute_name(name)
                .attribute_type(ScalarAttributeType::B)
                .build()
                .map_err(failed)
        };
        let key = |name: &str, kind| {
            KeySchemaElement::builder()
                .attribute_name(name)
                .key_type(kind)
                .build()
                .map_err(failed)
        };
        self.client
            .create_table()
            .table_name(&*self.name)
            .attribute_definitions(attribute(PK)?)
            .attribute_definitions(attribute(SK)?)
            .key_schema(key(PK, KeyType::Hash)?)
            .key_schema(key(SK, KeyType::Range)?)
            .billing_mode(BillingMode::PayPerRequest)
            .send()
            .await
            .map_err(failed)?;
        Ok(())
    }

    /// Whether the table exists.
    pub async fn exists(&self) -> Result<bool, EdgeError> {
        match self
            .client
            .describe_table()
            .table_name(&*self.name)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(error)
                if error
                    .as_service_error()
                    .is_some_and(|e| e.is_resource_not_found_exception()) =>
            {
                Ok(false)
            }
            Err(error) => Err(failed(error)),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn client(&self) -> &Client {
        &self.client
    }
}
