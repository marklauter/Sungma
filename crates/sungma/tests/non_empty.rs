//! A list that always holds at least one item.

use sungma::{non_empty::NonEmpty, nonempty};

#[test]
fn an_empty_list_is_refused() {
    assert_eq!(NonEmpty::<u8>::new(Vec::new()), None);
    assert_eq!(NonEmpty::new(vec![1]), Some(NonEmpty::one(1)));
}

#[test]
fn the_macro_lists_its_items_in_order() {
    let items = nonempty![1, 2, 3];
    assert_eq!(*items, [1, 2, 3]);
    assert_eq!(items, NonEmpty::with_rest(1, vec![2, 3]));
    assert_eq!(*nonempty![7; 2], [7, 7]);
}

#[test]
fn the_first_item_is_always_there() {
    let items = nonempty![1, 2, 3];
    assert_eq!(items.first_and_rest(), (&1, &[2, 3][..]));
    assert_eq!(nonempty![1].first_and_rest(), (&1, &[][..]));
}

#[test]
fn appending_and_mapping_keep_every_item() {
    let mut items = nonempty![1];
    items.append(nonempty![2, 3]);
    let doubled = items.clone().map(|item| item * 2);
    assert_eq!(*doubled, [2, 4, 6]);
    assert_eq!((&items).into_iter().sum::<i32>(), 6);
    assert_eq!(items.into_iter().collect::<Vec<_>>(), [1, 2, 3]);
}
