use std::{collections::HashMap, time::Duration};

use parking_lot::Mutex;

use crate::bff::nanos;

/// How long a signed-out session is remembered. A refresh that was out
/// when the person signed out lands well inside this, since a call to the
/// identity provider times out first.
const KEEP: Duration = Duration::from_secs(300);

/// Sessions ended by signing out, by id, so a refresh that lands after
/// sign-out doesn't save its session again.
pub(crate) struct SignedOut {
    ids: Mutex<HashMap<String, u64>>,
}

impl SignedOut {
    pub fn new() -> Self {
        Self {
            ids: Mutex::new(HashMap::new()),
        }
    }

    /// Remembers that session `id` signed out at `now`, in nanoseconds
    /// since the Unix epoch.
    pub fn add(&self, id: String, now: u64) {
        let mut ids = self.ids.lock();
        ids.retain(|_, at| now < at.saturating_add(nanos(KEEP)));
        ids.insert(id, now);
    }

    /// Whether session `id` signed out less than [`KEEP`] before `now`.
    pub fn contains(&self, id: &str, now: u64) -> bool {
        self.ids
            .lock()
            .get(id)
            .is_some_and(|at| now < at.saturating_add(nanos(KEEP)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: u64 = 1_000_000_000;

    #[test]
    fn a_signed_out_session_is_remembered_for_five_minutes() {
        let signed_out = SignedOut::new();
        signed_out.add("s1".to_owned(), 0);
        assert!(signed_out.contains("s1", 300 * SECOND - 1));
        assert!(!signed_out.contains("s1", 300 * SECOND));
        assert!(!signed_out.contains("s2", 0));
    }

    #[test]
    fn adding_forgets_sessions_past_five_minutes() {
        let signed_out = SignedOut::new();
        signed_out.add("s1".to_owned(), 0);
        signed_out.add("s2".to_owned(), 300 * SECOND - 1);
        assert_eq!(signed_out.ids.lock().len(), 2);
        signed_out.add("s3".to_owned(), 300 * SECOND);
        assert_eq!(signed_out.ids.lock().len(), 2);
    }
}
