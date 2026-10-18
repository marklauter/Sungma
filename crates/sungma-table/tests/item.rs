//! Reading an item's attributes: each as its kind, missing, or refused
//! when it holds another kind.

use std::collections::BTreeMap;

use sungma_table::item::{Item, Value};

fn item() -> Item {
    let map = BTreeMap::from([("1".to_owned(), Value::Number(1))]);
    Item::new()
        .with("n", Value::Number(7))
        .with("t", Value::Text("seven".to_owned()))
        .with("on", Value::Flag(true))
        .with("off", Value::Flag(false))
        .with("m", Value::Map(map))
}

#[test]
fn each_attribute_reads_as_its_kind() {
    let item = item();
    let map = BTreeMap::from([("1".to_owned(), Value::Number(1))]);
    assert_eq!(item.number("n"), Ok(Some(7)));
    assert_eq!(item.required_number("n"), Ok(7));
    assert_eq!(item.text("t"), Ok(Some("seven")));
    assert_eq!(item.required_text("t"), Ok("seven"));
    assert_eq!(item.flag("on"), Ok(true));
    assert_eq!(item.flag("off"), Ok(false));
    assert_eq!(item.map("m"), Ok(Some(&map)));
    assert_eq!(item.required_map("m"), Ok(&map));
}

/// A missing attribute reads as none, a missing flag as false, and a
/// required one is refused.
#[test]
fn a_missing_attribute_reads_as_none() {
    let item = Item::new();
    assert_eq!(item.number("x"), Ok(None));
    assert_eq!(item.text("x"), Ok(None));
    assert_eq!(item.flag("x"), Ok(false));
    assert_eq!(item.map("x"), Ok(None));
    assert!(item.required_number("x").is_err());
    assert!(item.required_text("x").is_err());
    assert!(item.required_map("x").is_err());
}

#[test]
fn an_attribute_of_another_kind_is_refused() {
    let item = item();
    assert!(item.number("t").is_err());
    assert!(item.text("n").is_err());
    assert!(item.flag("n").is_err());
    assert!(item.map("n").is_err());
    assert!(item.required_number("t").is_err());
    assert!(item.required_text("n").is_err());
    assert!(item.required_map("n").is_err());
}
