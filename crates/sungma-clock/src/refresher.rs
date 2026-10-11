//! Keeps a sampling clock's window fresh in the background.

use std::{any::Any, sync::Arc, time::Duration};

use tokio::{
    task::{self, JoinHandle},
    time::{self, Instant, MissedTickBehavior},
};

use crate::{ClockError, NtpClock, SystemClock, TimeSource};

/// How often the refresher takes a sample, NTP's shortest standard poll.
/// A server that sent Kiss-o'-Death `RATE` is held off longer on its own.
pub const POLL: Duration = Duration::from_secs(64);

/// A clock that takes fresh samples.
pub trait Refresh: Send + Sync + 'static {
    /// Takes a fresh sample. It may block on the network.
    fn refresh(&self) -> Result<(), ClockError>;
}

impl<T: TimeSource + 'static> Refresh for NtpClock<T> {
    fn refresh(&self) -> Result<(), ClockError> {
        self.resync()
    }
}

impl Refresh for SystemClock {
    fn refresh(&self) -> Result<(), ClockError> {
        SystemClock::refresh(self)
    }
}

/// Refreshes `clock` every [`POLL`], starting one poll from now, until the
/// returned task is aborted. Each refresh runs on a blocking thread, so
/// `now()` never waits on the network, and its result goes to `report`.
/// A failed refresh leaves the clock as it was, and the next one comes a
/// poll later. So does one that panics, reported as
/// [`ClockError::Panicked`].
pub fn refresher<C: Refresh>(
    clock: Arc<C>,
    report: impl Fn(Result<(), ClockError>) + Send + 'static,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticks = time::interval_at(Instant::now() + POLL, POLL);
        ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            ticks.tick().await;
            let clock = clock.clone();
            let refreshed = task::spawn_blocking(move || clock.refresh()).await;
            report(refreshed.unwrap_or_else(|failed| Err(panicked(failed))));
        }
    })
}

/// The error a refresh that didn't finish reports, with its panic's
/// message where it has one.
fn panicked(failed: task::JoinError) -> ClockError {
    let message = match failed.try_into_panic() {
        Ok(panic) => panic_message(panic),
        Err(failed) => failed.to_string(),
    };
    ClockError::Panicked {
        what: "a refresh",
        message,
    }
}

/// A panic's message, where it has one.
pub(crate) fn panic_message(panic: Box<dyn Any + Send>) -> String {
    match panic.downcast::<&str>() {
        Ok(text) => (*text).to_owned(),
        Err(panic) => panic
            .downcast::<String>()
            .map_or_else(|_| "no message".to_owned(), |text| *text),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, PoisonError};

    use super::*;

    /// A clock that counts its refreshes and fails every other one.
    #[derive(Default)]
    struct Counting(Mutex<u32>);

    impl Refresh for Counting {
        fn refresh(&self) -> Result<(), ClockError> {
            let mut count = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            *count += 1;
            if count.is_multiple_of(2) {
                return Err(ClockError::NoServers);
            }
            Ok(())
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_clock_is_refreshed_every_poll_and_through_failures() {
        let clock = Arc::new(Counting::default());
        let results = Arc::new(Mutex::new(Vec::new()));
        let seen = results.clone();
        let task = refresher(clock.clone(), move |result| {
            seen.lock().unwrap().push(result.is_ok());
        });
        let refreshes = || *clock.0.lock().unwrap();
        time::sleep(POLL - Duration::from_secs(1)).await;
        assert_eq!(refreshes(), 0);
        time::sleep(Duration::from_secs(2)).await;
        assert_eq!(refreshes(), 1);
        time::sleep(2 * POLL).await;
        assert_eq!(refreshes(), 3);
        assert_eq!(*results.lock().unwrap(), vec![true, false, true]);
        task.abort();
    }

    /// A clock whose first refresh panics.
    #[derive(Default)]
    struct Panicking(Mutex<u32>);

    impl Refresh for Panicking {
        fn refresh(&self) -> Result<(), ClockError> {
            let mut count = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            *count += 1;
            if *count == 1 {
                drop(count);
                panic!("the first refresh");
            }
            Ok(())
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_refresh_that_panics_is_reported_and_the_next_still_comes() {
        let results = Arc::new(Mutex::new(Vec::new()));
        let seen = results.clone();
        let task = refresher(Arc::new(Panicking::default()), move |result| {
            seen.lock()
                .unwrap()
                .push(result.map_err(|error| error.to_string()));
        });
        time::sleep(2 * POLL + Duration::from_secs(1)).await;
        assert_eq!(
            *results.lock().unwrap(),
            vec![
                Err("a refresh panicked: the first refresh".to_owned()),
                Ok(())
            ]
        );
        task.abort();
    }

    #[tokio::test]
    async fn a_panic_without_a_message_is_still_reported() {
        let failed = task::spawn_blocking(|| std::panic::panic_any(7))
            .await
            .unwrap_err();
        assert!(
            matches!(panicked(failed), ClockError::Panicked { message, .. } if message == "no message")
        );
        let formatted = task::spawn_blocking(|| panic!("{}", 7)).await.unwrap_err();
        assert!(
            matches!(panicked(formatted), ClockError::Panicked { message, .. } if message == "7")
        );
    }

    #[test]
    fn the_poll_is_64_seconds() {
        assert_eq!(POLL, Duration::from_secs(64));
    }
}
