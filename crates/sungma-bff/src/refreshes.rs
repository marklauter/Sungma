use std::{collections::HashMap, future::Future, sync::Arc};

use parking_lot::Mutex;
use tokio::sync::watch;

/// Refreshes running or still fresh, by the refresh token each one spends.
/// Requests that find a token about to expire at once share one refresh:
/// an identity provider that rotates refresh tokens accepts each only
/// once, and refuses the rest.
pub(crate) struct Refreshes<T> {
    entries: Arc<Mutex<HashMap<String, Entry<T>>>>,
}

enum Entry<T> {
    /// A refresh running in a task of its own. Its result arrives on the
    /// channel.
    Running(watch::Receiver<Option<T>>),
    /// A refresh's result, shared until `until`, in nanoseconds since the
    /// Unix epoch.
    Done { value: T, until: u64 },
}

impl<T: Clone + Send + Sync + 'static> Refreshes<T> {
    pub fn new() -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// The result of refreshing `refresh` at `now`: a result still shared,
    /// the result of a refresh already running, or of a new one that
    /// `start` makes. A new refresh runs in a task of its own, so it lands
    /// even if every request waiting on it goes away, and every waiting
    /// request gets its result.
    ///
    /// `start`'s future yields the result and when it stops being shared,
    /// or `None` to give it only to the requests already waiting. `None`
    /// comes back if the task ended without a result.
    pub async fn run<F>(&self, refresh: &str, now: u64, start: impl FnOnce() -> F) -> Option<T>
    where
        F: Future<Output = (T, Option<u64>)> + Send + 'static,
    {
        let mut result = {
            let mut entries = self.entries.lock();
            entries.retain(|_, entry| match entry {
                Entry::Done { until, .. } => now < *until,
                // A task that ended without a result leaves its entry
                // running; dropping it lets the next request start over.
                Entry::Running(result) => result.has_changed().is_ok(),
            });
            match entries.get(refresh) {
                Some(Entry::Done { value, .. }) => return Some(value.clone()),
                Some(Entry::Running(result)) => result.clone(),
                None => {
                    let (sender, result) = watch::channel(None);
                    entries.insert(refresh.to_owned(), Entry::Running(result.clone()));
                    let entries = Arc::clone(&self.entries);
                    let key = refresh.to_owned();
                    let refreshing = start();
                    tokio::spawn(async move {
                        let (value, until) = refreshing.await;
                        match until {
                            Some(until) => {
                                let done = Entry::Done {
                                    value: value.clone(),
                                    until,
                                };
                                entries.lock().insert(key, done);
                            }
                            None => {
                                entries.lock().remove(&key);
                            }
                        }
                        sender.send_replace(Some(value));
                    });
                    result
                }
            }
        };
        result
            .wait_for(Option::is_some)
            .await
            .ok()
            .and_then(|value| value.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::sync::Notify;

    use super::*;

    /// Refreshes whose `start` counts its calls and waits for `go`.
    struct Counted {
        refreshes: Refreshes<u32>,
        started: Arc<AtomicUsize>,
        go: Arc<Notify>,
    }

    impl Counted {
        fn new() -> Self {
            Self {
                refreshes: Refreshes::new(),
                started: Arc::new(AtomicUsize::new(0)),
                go: Arc::new(Notify::new()),
            }
        }

        /// Refreshes `key` at `now`; a new refresh waits for `go` when
        /// `wait`, and yields `value`, shared until `until`.
        async fn run(
            &self,
            key: &str,
            now: u64,
            wait: bool,
            value: u32,
            until: Option<u64>,
        ) -> Option<u32> {
            let started = Arc::clone(&self.started);
            let go = Arc::clone(&self.go);
            self.refreshes
                .run(key, now, move || async move {
                    started.fetch_add(1, Ordering::SeqCst);
                    if wait {
                        go.notified().await;
                    }
                    (value, until)
                })
                .await
        }

        fn started(&self) -> usize {
            self.started.load(Ordering::SeqCst)
        }
    }

    #[tokio::test]
    async fn requests_at_once_share_one_refresh() {
        let c = Counted::new();
        let go = Arc::clone(&c.go);
        let (a, b, ()) = tokio::join!(
            c.run("r1", 0, true, 1, Some(10)),
            c.run("r1", 0, true, 2, Some(10)),
            async move {
                tokio::task::yield_now().await;
                go.notify_one();
            },
        );
        assert_eq!((a, b), (Some(1), Some(1)));
        assert_eq!(c.started(), 1);
    }

    #[tokio::test]
    async fn a_result_is_shared_until_its_time() {
        let c = Counted::new();
        assert_eq!(c.run("r1", 0, false, 1, Some(10)).await, Some(1));
        assert_eq!(c.run("r1", 9, false, 2, Some(20)).await, Some(1));
        assert_eq!(c.started(), 1);
        assert_eq!(c.run("r1", 10, false, 3, Some(20)).await, Some(3));
        assert_eq!(c.started(), 2);
    }

    #[tokio::test]
    async fn a_result_without_a_time_goes_only_to_its_waiters() {
        let c = Counted::new();
        assert_eq!(c.run("r1", 0, false, 1, None).await, Some(1));
        assert_eq!(c.run("r1", 0, false, 2, None).await, Some(2));
        assert_eq!(c.started(), 2);
    }

    #[tokio::test]
    async fn other_refresh_tokens_refresh_apart() {
        let c = Counted::new();
        assert_eq!(c.run("r1", 0, false, 1, Some(10)).await, Some(1));
        assert_eq!(c.run("r2", 0, false, 2, Some(10)).await, Some(2));
        assert_eq!(c.started(), 2);
    }

    async fn fail() -> (u32, Option<u64>) {
        panic!("the refresh task fails")
    }

    #[tokio::test]
    async fn a_refresh_whose_task_ends_without_a_result_starts_over() {
        let refreshes = Refreshes::<u32>::new();
        let failed = refreshes.run("r1", 0, fail).await;
        assert_eq!(failed, None);
        let again = refreshes.run("r1", 0, || async { (2, Some(10)) }).await;
        assert_eq!(again, Some(2));
    }
}
