//! A clock a test sets by hand.

use std::future::Future;

use sungma::clock::{Clock, Reading};
use tokio::sync::watch;

/// Reads whatever the test last set. A wait resolves when a later
/// [`ManualClock::set`] moves `settled` past the stamped revision, so a test can
/// hold a write in its commit-wait and then release it.
#[derive(Debug)]
pub struct ManualClock {
    reading: watch::Sender<Reading>,
}

impl ManualClock {
    pub fn new(reading: Reading) -> Self {
        Self {
            reading: watch::Sender::new(reading),
        }
    }

    pub fn set(&self, reading: Reading) {
        self.reading.send_replace(reading);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Reading {
        *self.reading.borrow()
    }

    fn wait(&self, stamped: Reading) -> impl Future<Output = ()> + Send {
        let mut reading = self.reading.subscribe();
        async move {
            // Only a dropped sender fails the wait, and the sender lives as
            // long as the clock the caller borrows.
            let _ = reading.wait_for(|now| now.settled > stamped.revision).await;
        }
    }
}
