//! A clock a test sets by hand.

use std::future::Future;

use sungma::clock::{Clock, Interval, Revision};
use tokio::sync::watch;

/// Reads whatever interval the test last set. A wait resolves when a later
/// [`ManualClock::set`] moves `earliest` past its revision, so a test can
/// hold a write in its commit-wait and then release it.
#[derive(Debug)]
pub struct ManualClock {
    interval: watch::Sender<Interval>,
}

impl ManualClock {
    pub fn new(interval: Interval) -> Self {
        Self {
            interval: watch::Sender::new(interval),
        }
    }

    pub fn set(&self, interval: Interval) {
        self.interval.send_replace(interval);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Interval {
        *self.interval.borrow()
    }

    fn wait_past(&self, revision: Revision) -> impl Future<Output = ()> + Send {
        let mut interval = self.interval.subscribe();
        async move {
            // Only a dropped sender fails the wait, and the sender lives as
            // long as the clock the caller borrows.
            let _ = interval.wait_for(|now| now.earliest > revision).await;
        }
    }
}
