//! The head: one row holding the revision of the latest write, of a fact
//! or of a namespace version. Every write moves it, on condition it still
//! holds what the writer read, so writes land in revision order.

use sungma::{revision::Revision, store::StoreError};
use sungma_table::{
    item::{Item, Value},
    key::Key,
    read::{Consistency, ItemReader},
    write::{Expect, Step},
};

use crate::{failed, tags::HEAD};

const REVISION: &str = "revision";

pub fn key() -> Key {
    Key::new(HEAD, 0u8)
}

/// The head's revision, `None` before the first write.
pub async fn read<R: ItemReader>(
    items: &R,
    consistency: Consistency,
) -> Result<Option<Revision>, StoreError> {
    let row = items.get(&key(), consistency).await.map_err(failed)?;
    row.map(|row| row.required_number(REVISION).map(Revision).map_err(failed))
        .transpose()
}

/// Moves the head from `seen`, what the writer read, to `at`.
pub fn step(seen: Option<Revision>, at: Revision) -> Step {
    let item = Item::new().with(REVISION, Value::Number(at.0));
    match seen {
        None => Step::Insert { key: key(), item },
        Some(seen) => Step::Update {
            key: key(),
            set: item,
            remove: Vec::new(),
            expect: vec![Expect::Equals(REVISION.to_owned(), Value::Number(seen.0))],
        },
    }
}
