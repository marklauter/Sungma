//! A counter, for a single node.

use std::{
    future::{self, Future},
    sync::atomic::{AtomicU64, Ordering},
};

use sungma::clock::{Clock, Reading, Revision};

/// A clock with no uncertainty: each reading is one past the last, with
/// `settled` equal to `revision`, so a wait never sleeps. Revisions count
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
    fn now(&self) -> Reading {
        let next = Revision(self.last.fetch_add(1, Ordering::SeqCst) + 1);
        Reading {
            settled: next,
            revision: next,
        }
    }

    fn wait(&self, stamped: Reading) -> impl Future<Output = ()> + Send {
        self.last.fetch_max(stamped.revision.0, Ordering::SeqCst);
        future::ready(())
    }
}
