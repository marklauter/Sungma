use std::{collections::HashMap, sync::Arc, time::Duration};

use parking_lot::Mutex;
use tokio::sync::OnceCell;

/// How long a refresh's result is kept for requests that overlapped it.
/// They arrive within moments; a call to the identity provider times out
/// well inside this.
const KEEP: Duration = Duration::from_secs(300);

/// Refreshes in flight or just finished, by the refresh token each one
/// spends. Requests that find a token about to expire at once share one
/// refresh: an identity provider that rotates refresh tokens accepts each
/// only once, and refuses the rest.
pub(crate) struct Refreshes<T> {
    entries: Mutex<HashMap<String, Entry<T>>>,
}

struct Entry<T> {
    started: u64,
    result: Arc<OnceCell<T>>,
}

impl<T> Refreshes<T> {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// The cell that holds the refresh of `refresh`, new or shared, at
    /// `now` in nanoseconds since the Unix epoch. Cells older than
    /// [`KEEP`] are dropped.
    pub fn cell(&self, refresh: &str, now: u64) -> Arc<OnceCell<T>> {
        let keep = u64::try_from(KEEP.as_nanos()).unwrap_or(u64::MAX);
        let mut entries = self.entries.lock();
        entries.retain(|_, entry| now < entry.started.saturating_add(keep));
        let entry = entries.entry(refresh.to_owned()).or_insert_with(|| Entry {
            started: now,
            result: Arc::new(OnceCell::new()),
        });
        Arc::clone(&entry.result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: u64 = 1_000_000_000;

    #[test]
    fn one_refresh_token_shares_one_cell() {
        let refreshes = Refreshes::<u8>::new();
        let first = refreshes.cell("r1", 0);
        assert!(Arc::ptr_eq(&first, &refreshes.cell("r1", SECOND)));
        assert!(!Arc::ptr_eq(&first, &refreshes.cell("r2", SECOND)));
    }

    #[test]
    fn a_cell_is_dropped_after_five_minutes() {
        let refreshes = Refreshes::<u8>::new();
        let first = refreshes.cell("r1", 0);
        assert!(Arc::ptr_eq(&first, &refreshes.cell("r1", 300 * SECOND - 1)));
        assert!(!Arc::ptr_eq(&first, &refreshes.cell("r1", 300 * SECOND)));
    }
}
