//! A clock a test sets by hand.

use std::future::Future;

use crate::{Clock, ClockFault, Reading};
use tokio::sync::watch;

/// Reads whatever the test last set: a reading, or a fault. A wait
/// resolves when a later [`ManualClock::set`] moves `settled` past the
/// stamped revision, so a test can hold a write in its commit-wait and
/// then release it, or ends with the fault a later [`ManualClock::fault`]
/// sets.
#[derive(Debug)]
pub struct ManualClock {
    state: watch::Sender<Result<Reading, ClockFault>>,
}

impl ManualClock {
    pub fn new(reading: Reading) -> Self {
        Self {
            state: watch::Sender::new(Ok(reading)),
        }
    }

    pub fn set(&self, reading: Reading) {
        self.state.send_modify(|state| *state = Ok(reading));
    }

    pub fn fault(&self, fault: ClockFault) {
        self.state.send_modify(|state| *state = Err(fault));
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Result<Reading, ClockFault> {
        *self.state.borrow()
    }

    fn wait(&self, stamped: Reading) -> impl Future<Output = Result<(), ClockFault>> + Send {
        let mut state = self.state.subscribe();
        async move {
            let done = |now: &Result<Reading, ClockFault>| match now {
                Ok(now) => now.settled > stamped.revision,
                Err(_) => true,
            };
            match state.wait_for(done).await {
                Ok(now) => now.map(|_| ()),
                // Only a dropped sender fails the wait, and the sender
                // lives as long as the clock the caller borrows.
                Err(_) => Ok(()),
            }
        }
    }
}
