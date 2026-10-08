//! The system clock, and the bound a sample of it carries.

use std::{
    sync::{Mutex, PoisonError},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use sungma::clock::{Clock, Interval, Revision};

/// The most a disciplined clock is assumed to drift, in parts per million:
/// the rate the Linux kernel grows its own error bound by.
const MAX_DRIFT_PPM: u128 = 500;

/// Nanoseconds since the Unix epoch, by the system clock.
pub(crate) fn wall_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| saturate(since.as_nanos()))
}

fn saturate(nanos: u128) -> u64 {
    u64::try_from(nanos).unwrap_or(u64::MAX)
}

/// How far a clock may have drifted in `elapsed`, in nanoseconds.
pub(crate) fn drift(elapsed: Duration) -> u64 {
    saturate(elapsed.as_nanos() * MAX_DRIFT_PPM / 1_000_000)
}

/// The interval `bound` nanoseconds either side of `at`.
pub(crate) fn interval(at: u64, bound: u64) -> Interval {
    Interval {
        earliest: Revision(at.saturating_sub(bound)),
        latest: Revision(at.saturating_add(bound)),
    }
}

/// How long until a clock reading `earliest` passes `revision`, or `None`
/// once it has.
pub(crate) fn until_past(earliest: Revision, revision: Revision) -> Option<Duration> {
    (earliest <= revision).then(|| Duration::from_nanos(revision.0 - earliest.0 + 1))
}

/// Sleeps until `clock` reports an `earliest` past `revision`. A bound
/// that grows during a sleep takes another.
pub(crate) async fn wait_past(clock: &(impl Clock + Sync), revision: Revision) {
    while let Some(wait) = until_past(clock.now().earliest, revision) {
        tokio::time::sleep(wait).await;
    }
}

/// The system clock's error when sampled: true time is the system clock
/// plus `offset` nanoseconds, give or take `bound`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Sample {
    pub(crate) offset: i64,
    pub(crate) bound: u64,
}

impl Sample {
    /// True time when the system clock reads `wall`, `elapsed` after the
    /// sample was taken.
    pub(crate) fn interval(self, wall: u64, elapsed: Duration) -> Interval {
        interval(
            wall.saturating_add_signed(self.offset),
            self.bound.saturating_add(drift(elapsed)),
        )
    }
}

/// The latest sample and when it was taken. Its bound grows with drift
/// until a fresh sample replaces it.
#[derive(Debug)]
pub(crate) struct Sampled(Mutex<(Sample, Instant)>);

impl Sampled {
    pub(crate) fn new(sample: Sample) -> Self {
        Self(Mutex::new((sample, Instant::now())))
    }

    pub(crate) fn replace(&self, sample: Sample) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = (sample, Instant::now());
    }

    pub(crate) fn now(&self) -> Interval {
        let (sample, taken) = *self.0.lock().unwrap_or_else(PoisonError::into_inner);
        sample.interval(wall_nanos(), taken.elapsed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn width(interval: Interval) -> u64 {
        interval.latest.0 - interval.earliest.0
    }

    #[test]
    fn the_wall_clock_reads_the_system_clock() {
        let system = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        assert!(wall_nanos().abs_diff(saturate(system.as_nanos())) < 1_000_000_000);
    }

    #[test]
    fn drift_is_500_parts_per_million() {
        assert_eq!(drift(Duration::from_secs(2)), 1_000_000);
        assert_eq!(drift(Duration::ZERO), 0);
        assert_eq!(drift(Duration::MAX), u64::MAX);
    }

    #[test]
    fn an_interval_saturates_at_both_ends() {
        let between = |earliest, latest| Interval {
            earliest: Revision(earliest),
            latest: Revision(latest),
        };
        assert_eq!(interval(10, 3), between(7, 13));
        assert_eq!(interval(2, 5), between(0, 7));
        assert_eq!(interval(u64::MAX - 1, 5), between(u64::MAX - 6, u64::MAX));
    }

    #[test]
    fn a_wait_lasts_until_earliest_is_one_past() {
        let wait = |earliest, revision| until_past(Revision(earliest), Revision(revision));
        assert_eq!(wait(10, 14), Some(Duration::from_nanos(5)));
        assert_eq!(wait(14, 14), Some(Duration::from_nanos(1)));
        assert_eq!(wait(15, 14), None);
    }

    #[test]
    fn a_sample_shifts_by_its_offset_and_widens_with_drift() {
        let behind = Sample {
            offset: 100,
            bound: 7,
        };
        assert_eq!(
            behind.interval(1_000, Duration::from_micros(10)),
            interval(1_100, 12)
        );
        let ahead = Sample {
            offset: -100,
            bound: 0,
        };
        assert_eq!(ahead.interval(1_000, Duration::ZERO), interval(900, 0));
    }

    #[test]
    fn a_replaced_sample_is_the_one_read() {
        let sampled = Sampled::new(Sample {
            offset: 0,
            bound: 1_000_000_000_000,
        });
        assert!(width(sampled.now()) >= 2_000_000_000_000);
        sampled.replace(Sample {
            offset: 0,
            bound: 0,
        });
        let now = sampled.now();
        assert!(width(now) < 1_000_000_000);
        assert!(now.earliest.0.abs_diff(wall_nanos()) < 1_000_000_000);
    }
}
