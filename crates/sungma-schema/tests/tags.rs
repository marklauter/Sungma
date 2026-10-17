//! Every kind of row has a partition tag of its own.

use std::collections::HashSet;

use sungma_schema::tags::ALL;

#[test]
fn no_two_kinds_of_row_share_a_tag() {
    let distinct: HashSet<_> = ALL.iter().collect();
    assert_eq!(distinct.len(), ALL.len());
}
