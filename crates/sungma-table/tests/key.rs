//! Keys encode so they sort as their parts do, and read back unambiguously.

use proptest::prelude::*;
use sungma_table::key::{Key, Parts, encode};

#[test]
fn text_holding_a_zero_byte_is_not_two_texts() {
    assert_ne!(encode(("a\0b",)), encode(("a", "b")));
    let key = encode(("a\0b", 7u32));
    let mut parts = Parts::new(&key);
    assert_eq!(parts.text().as_deref(), Some("a\0b"));
    assert_eq!(parts.u32(), Some(7));
    assert_eq!(parts.end(()), Some(()));
}

#[test]
fn a_key_debugs_as_hex() {
    let key = Key {
        partition: vec![0x0a, 0xff],
        sort: vec![0x01],
    };
    assert_eq!(format!("{key:?}"), "Key(0aff/01)");
}

proptest! {
    #[test]
    fn texts_sort_and_read_back_as_they_are(a in ".{0,8}", b in ".{0,8}", n: u64, m: u64) {
        let (x, y) = (encode((a.as_str(), n)), encode((b.as_str(), m)));
        prop_assert_eq!(x.cmp(&y), (a.as_bytes(), n).cmp(&(b.as_bytes(), m)));
        let mut parts = Parts::new(&x);
        let text = parts.text();
        prop_assert_eq!(text.as_deref(), Some(a.as_str()));
        prop_assert_eq!(parts.u64(), Some(n));
        prop_assert_eq!(parts.end(()), Some(()));
    }
}
