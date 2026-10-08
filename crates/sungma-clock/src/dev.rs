//! A counter, for a single node.

use std::{
    future::{self, Future},
    sync::atomic::{AtomicU64, Ordering},
};

use sungma::clock::{Clock, Interval, Revision};

/// A clock with no uncertainty: each reading is one past the last, with
/// `earliest` equal to `latest`, so a wait never sleeps. Revisions count
/// readings rather than nanoseconds. A node that restarts starts the
/// counter after its store's latest revision, so it never issues one twice.
#[derive(Debug, Default)]
pub struct DevClock {
    last: AtomicU64,
}

impl DevClock {
    /// A clock whose first reading is one past `last`.
    pub fn starting_after(last: Revision) -> Self {
        Self {
            last: AtomicU64::new(last.0),
        }
    }
}

impl Clock for DevClock {
    fn now(&self) -> Interval {
        let next = Revision(self.last.fetch_add(1, Ordering::SeqCst) + 1);
        Interval {
            earliest: next,
            latest: next,
        }
    }

    fn wait_past(&self, revision: Revision) -> impl Future<Output = ()> + Send {
        self.last.fetch_max(revision.0, Ordering::SeqCst);
        future::ready(())
    }
}
