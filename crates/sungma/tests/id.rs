//! Ids print bare, for messages whose names aren't at hand.

use sungma::id::{NamespaceId, RelationId};

#[test]
fn an_id_prints_its_number() {
    assert_eq!(NamespaceId(7).to_string(), "7");
    assert_eq!(RelationId(42).to_string(), "42");
}
