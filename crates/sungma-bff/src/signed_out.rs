use std::{
    collections::{HashMap, VecDeque},
    time::Duration,
};

use parking_lot::Mutex;

use crate::bff::nanos;

/// How long a signed-out session is remembered, at the least. A request
/// that was running when the person signed out ends well inside this.
const KEEP: Duration = Duration::from_secs(300);

/// Sessions ended by signing out, by id, so a request that was running
/// when the person signed out doesn't save its session again.
pub(crate) struct SignedOut {
    /// [`KEEP`], or the time a call to the identity provider may take if
    /// that is longer, so a refresh out at sign-out lands inside it.
    keep: u64,
    ids: Mutex<Ids>,
}

/// The signed-out ids and when each signed out, and the same in the order
/// they signed out, oldest first, so forgetting them doesn't scan them
/// all.
#[derive(Default)]
struct Ids {
    at: HashMap<String, u64>,
    order: VecDeque<(u64, String)>,
}

impl SignedOut {
    /// Remembers sign-outs long enough for a call to the identity provider
    /// that takes `timeout` to land inside it.
    pub fn new(timeout: Duration) -> Self {
        Self {
            keep: nanos(KEEP.max(timeout)),
            ids: Mutex::new(Ids::default()),
        }
    }

    /// Remembers that session `id` signed out at `now`, in nanoseconds
    /// since the Unix epoch.
    pub fn add(&self, id: String, now: u64) {
        let mut ids = self.ids.lock();
        while let Some((at, _)) = ids.order.front() {
            if now < at.saturating_add(self.keep) {
                break;
            }
            if let Some((at, id)) = ids.order.pop_front()
                && ids.at.get(&id) == Some(&at)
            {
                ids.at.remove(&id);
            }
        }
        ids.at.insert(id.clone(), now);
        ids.order.push_back((now, id));
    }

    /// Whether session `id` signed out less than the time kept before
    /// `now`.
    pub fn contains(&self, id: &str, now: u64) -> bool {
        self.ids
            .lock()
            .at
            .get(id)
            .is_some_and(|at| now < at.saturating_add(self.keep))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: u64 = 1_000_000_000;

    #[test]
    fn a_signed_out_session_is_remembered_for_five_minutes() {
        let signed_out = SignedOut::new(Duration::from_secs(30));
        signed_out.add("s1".to_owned(), 0);
        assert!(signed_out.contains("s1", 300 * SECOND - 1));
        assert!(!signed_out.contains("s1", 300 * SECOND));
        assert!(!signed_out.contains("s2", 0));
    }

    #[test]
    fn a_long_timeout_keeps_a_signed_out_session_longer() {
        let signed_out = SignedOut::new(Duration::from_secs(600));
        signed_out.add("s1".to_owned(), 0);
        assert!(signed_out.contains("s1", 600 * SECOND - 1));
        assert!(!signed_out.contains("s1", 600 * SECOND));
    }

    #[test]
    fn adding_forgets_sessions_past_five_minutes() {
        let signed_out = SignedOut::new(Duration::from_secs(30));
        signed_out.add("s1".to_owned(), 0);
        signed_out.add("s2".to_owned(), 300 * SECOND - 1);
        assert_eq!(signed_out.ids.lock().at.len(), 2);
        signed_out.add("s3".to_owned(), 300 * SECOND);
        let ids = signed_out.ids.lock();
        assert_eq!(ids.at.len(), 2);
        assert_eq!(ids.order.len(), 2);
    }

    #[test]
    fn a_session_signed_out_again_is_remembered_from_the_second_time() {
        let signed_out = SignedOut::new(Duration::from_secs(30));
        signed_out.add("s1".to_owned(), 0);
        signed_out.add("s1".to_owned(), 200 * SECOND);
        // Forgetting the first sign-out leaves the second.
        signed_out.add("s2".to_owned(), 300 * SECOND);
        assert!(signed_out.contains("s1", 499 * SECOND));
    }
}
