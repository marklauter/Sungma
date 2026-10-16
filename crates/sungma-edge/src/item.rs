//! Items: named attributes, each item holding whatever names it needs.

use std::collections::BTreeMap;

use crate::EdgeError;

/// An attribute's value.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Value {
    Number(u64),
    Text(String),
    Flag(bool),
    Map(BTreeMap<String, Value>),
}

/// An item's attributes, by name.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Item(pub BTreeMap<String, Value>);

impl Item {
    pub fn new() -> Self {
        Self::default()
    }

    /// The item with `name` set to `value`.
    pub fn with(mut self, name: &str, value: Value) -> Self {
        self.0.insert(name.to_owned(), value);
        self
    }

    pub fn get(&self, name: &str) -> Option<&Value> {
        self.0.get(name)
    }

    /// The number `name` holds, `None` when the item hasn't it, refused
    /// when it holds something else.
    pub fn number(&self, name: &str) -> Result<Option<u64>, EdgeError> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::Number(number)) => Ok(Some(*number)),
            Some(_) => Err(not(name, "a number")),
        }
    }

    pub fn text(&self, name: &str) -> Result<Option<&str>, EdgeError> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::Text(text)) => Ok(Some(text)),
            Some(_) => Err(not(name, "text")),
        }
    }

    /// A flag the item hasn't is false.
    pub fn flag(&self, name: &str) -> Result<bool, EdgeError> {
        match self.get(name) {
            None => Ok(false),
            Some(Value::Flag(flag)) => Ok(*flag),
            Some(_) => Err(not(name, "a flag")),
        }
    }

    pub fn map(&self, name: &str) -> Result<Option<&BTreeMap<String, Value>>, EdgeError> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::Map(map)) => Ok(Some(map)),
            Some(_) => Err(not(name, "a map")),
        }
    }

    /// Like [`Item::number`], refused when the item hasn't it.
    pub fn required_number(&self, name: &str) -> Result<u64, EdgeError> {
        self.number(name)?.ok_or_else(|| missing(name))
    }

    pub fn required_text(&self, name: &str) -> Result<&str, EdgeError> {
        self.text(name)?.ok_or_else(|| missing(name))
    }

    pub fn required_map(&self, name: &str) -> Result<&BTreeMap<String, Value>, EdgeError> {
        self.map(name)?.ok_or_else(|| missing(name))
    }
}

fn not(name: &str, kind: &str) -> EdgeError {
    EdgeError(format!("attribute '{name}' isn't {kind}"))
}

fn missing(name: &str) -> EdgeError {
    EdgeError(format!("attribute '{name}' is missing"))
}
