//! The system clock, and the samples that bound it.
//!
//! A sample measures how far the system clock is from true time. From then
//! on, true time is read off the monotonic clock, which a step of the
//! system clock doesn't move, and the bound grows by the drift allowance.
//! Comparing the two clocks catches a step: a small one widens the bound
//! by its size, and a large one is a jump that faults the clock until a
//! fresh sample.

use std::{
    collections::VecDeque,
    sync::{Mutex, PoisonError},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use sungma::clock::{Clock, ClockFault, Reading, Revision};

use crate::{Moment, TimeSource};

/// The most a disciplined clock is assumed to drift, in parts per million:
/// the rate the Linux kernel grows its own error bound by.
const MAX_DRIFT_PPM: u128 = 500;

/// A clock whose bound is wider than this is unsynchronized: a write would
/// wait seconds to be acknowledged.
pub(crate) const MAX_BOUND: u64 = 1_000_000_000;

/// The most the system clock may move against the monotonic clock before
/// it counts as a jump, 10 ms.
const MAX_STEP: u64 = 10_000_000;

/// How many samples a clock keeps, as NTP's clock filter does.
const WINDOW: usize = 8;

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

/// The reading `bound` nanoseconds either side of `at`, or
/// [`ClockFault::Unsynchronized`] when the bound is past [`MAX_BOUND`].
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn reading(at: u64, bound: u64) -> Result<Reading, ClockFault> {
    if bound > MAX_BOUND {
        return Err(ClockFault::Unsynchronized);
    }
    Ok(Reading {
        settled: Revision(at.saturating_sub(bound)),
        revision: Revision(at.saturating_add(bound)),
    })
}

/// How much `step`, how far the system clock moved against the monotonic
/// clock, widens a bound: its size plus `slack`, the uncertainty of the
/// moments it was measured between. [`ClockFault::Jumped`] when it is past
/// [`MAX_STEP`] by more than the slack.
pub(crate) fn step(step: i128, slack: u64) -> Result<u64, ClockFault> {
    let size = step.unsigned_abs();
    if size > u128::from(MAX_STEP) + u128::from(slack) {
        let by = step.clamp(i64::MIN.into(), i64::MAX.into()) as i64;
        return Err(ClockFault::Jumped { by });
    }
    Ok(saturate(size).saturating_add(slack))
}

/// Nanoseconds of `elapsed`, signed.
pub(crate) fn nanos(elapsed: Duration) -> i128 {
    i128::from(saturate(elapsed.as_nanos()))
}

/// How long until a clock whose settled time is `settled` passes
/// `revision`, or `None` once it has.
pub(crate) fn until_past(settled: Revision, revision: Revision) -> Option<Duration> {
    (settled <= revision).then(|| Duration::from_nanos(revision.0 - settled.0 + 1))
}

/// Sleeps until `clock`'s settled time is past `stamped.revision`. A bound
/// that grows during a sleep takes another, and a fault ends the wait.
pub(crate) async fn wait(clock: &(impl Clock + Sync), stamped: Reading) -> Result<(), ClockFault> {
    while let Some(wait) = until_past(clock.now()?.settled, stamped.revision) {
        tokio::time::sleep(wait).await;
    }
    Ok(())
}

/// The system clock's error when sampled: true time is the system clock
/// plus `offset` nanoseconds, give or take `bound`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Sample {
    pub(crate) offset: i64,
    pub(crate) bound: u64,
}

/// A sample, with both clocks read when it was taken.
#[derive(Clone, Copy, Debug)]
struct Taken {
    sample: Sample,
    at: Moment,
}

impl Taken {
    fn read(sample: Sample, time: &impl TimeSource) -> Self {
        Self {
            sample,
            at: time.moment(),
        }
    }

    /// True time as this sample bounds it, its low and high ends, at
    /// moment `now`.
    fn range(&self, now: Moment) -> Result<(i128, i128), ClockFault> {
        let elapsed = Duration::from_nanos(now.monotonic.saturating_sub(self.at.monotonic));
        let elapsed_nanos = nanos(elapsed);
        let moved = i128::from(now.wall) - i128::from(self.at.wall);
        let slack = self.at.uncertainty.saturating_add(now.uncertainty);
        let widen = step(moved - elapsed_nanos, slack)?;
        let center = i128::from(self.at.wall) + i128::from(self.sample.offset) + elapsed_nanos;
        let bound = i128::from(self.sample.bound) + i128::from(drift(elapsed)) + i128::from(widen);
        Ok((center - bound, center + bound))
    }
}

/// Where every range meets, or `None` when there are none or two don't
/// overlap.
fn meet(ranges: impl IntoIterator<Item = (i128, i128)>) -> Option<(i128, i128)> {
    let (low, high) = ranges
        .into_iter()
        .reduce(|(low, high), (l, h)| (low.max(l), high.min(h)))?;
    (low <= high).then_some((low, high))
}

/// The reading spanning `low` to `high`.
fn spanning(low: i128, high: i128) -> Result<Reading, ClockFault> {
    if (high - low) / 2 > i128::from(MAX_BOUND) {
        return Err(ClockFault::Unsynchronized);
    }
    let clamp = |at: i128| u64::try_from(at.max(0)).unwrap_or(u64::MAX);
    Ok(Reading {
        settled: Revision(clamp(low)),
        revision: Revision(clamp(high)),
    })
}

/// The last few samples, read against a time source. A reading is where
/// all of them meet, so one sample with a slow round trip doesn't widen
/// the bound.
#[derive(Debug)]
pub(crate) struct Sampled<T> {
    time: T,
    window: Mutex<VecDeque<Taken>>,
}

impl<T: TimeSource> Sampled<T> {
    pub(crate) fn new(sample: Sample, time: T) -> Self {
        let first = Taken::read(sample, &time);
        Self {
            time,
            window: Mutex::new(VecDeque::from([first])),
        }
    }

    pub(crate) fn time(&self) -> &T {
        &self.time
    }

    /// Adds a fresh sample. Samples from before a jump leave the window.
    /// [`ClockFault::Drifted`] when the sample doesn't overlap the rest,
    /// which then leave the window too, so the clock reads from the fresh
    /// sample alone.
    pub(crate) fn add(&self, sample: Sample) -> Result<(), ClockFault> {
        let fresh = Taken::read(sample, &self.time);
        let mut window = self.window.lock().unwrap_or_else(PoisonError::into_inner);
        let ranges: Vec<_> = window.iter().map(|taken| taken.range(fresh.at)).collect();
        let mut kept = ranges.iter().map(Result::is_ok);
        window.retain(|_| kept.next().unwrap_or(false));
        let earlier = ranges.into_iter().filter_map(Result::ok);
        let ours = fresh.range(fresh.at)?;
        let agrees = window.is_empty() || meet(earlier.chain([ours])).is_some();
        if !agrees {
            window.clear();
        }
        if window.len() == WINDOW {
            window.pop_front();
        }
        window.push_back(fresh);
        if agrees {
            Ok(())
        } else {
            Err(ClockFault::Drifted)
        }
    }

    /// Where the window's samples meet. [`ClockFault::Jumped`] when the
    /// system clock jumped since the newest sample.
    pub(crate) fn now(&self) -> Result<Reading, ClockFault> {
        let now = self.time.moment();
        let window = self.window.lock().unwrap_or_else(PoisonError::into_inner);
        let newest = window.back().expect("a window always holds a sample");
        newest.range(now)?;
        let ranges = window.iter().filter_map(|taken| taken.range(now).ok());
        let (low, high) = meet(ranges).ok_or(ClockFault::Drifted)?;
        spanning(low, high)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OsTime;

    const MS: u64 = 1_000_000;

    fn width(reading: Reading) -> u64 {
        reading.revision.0 - reading.settled.0
    }

    fn sample(offset: i64, bound: u64) -> Sample {
        Sample { offset, bound }
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
    fn a_reading_saturates_at_both_ends() {
        let between = |settled, revision| {
            Ok(Reading {
                settled: Revision(settled),
                revision: Revision(revision),
            })
        };
        assert_eq!(reading(10, 3), between(7, 13));
        assert_eq!(reading(2, 5), between(0, 7));
        assert_eq!(reading(u64::MAX - 1, 5), between(u64::MAX - 6, u64::MAX));
    }

    #[test]
    fn a_bound_past_a_second_is_unsynchronized() {
        assert!(reading(5 * MAX_BOUND, MAX_BOUND).is_ok());
        assert_eq!(
            reading(5 * MAX_BOUND, MAX_BOUND + 1),
            Err(ClockFault::Unsynchronized)
        );
        let max = i128::from(MAX_BOUND);
        assert!(spanning(0, 2 * max).is_ok());
        assert_eq!(spanning(0, 2 * max + 2), Err(ClockFault::Unsynchronized));
    }

    #[test]
    fn a_spanning_reading_clamps_to_revisions() {
        let reading = spanning(-5, 7).unwrap();
        assert_eq!(
            (reading.settled, reading.revision),
            (Revision(0), Revision(7))
        );
        let late = i128::from(u64::MAX);
        let reading = spanning(late - 3, late + 3).unwrap();
        assert_eq!(
            (reading.settled, reading.revision),
            (Revision(u64::MAX - 3), Revision(u64::MAX))
        );
    }

    #[test]
    fn a_step_past_10_ms_is_a_jump() {
        let max = i128::from(MAX_STEP);
        assert_eq!(step(max, 0), Ok(MAX_STEP));
        assert_eq!(step(-max, 0), Ok(MAX_STEP));
        assert_eq!(step(max + 1, 0), Err(ClockFault::Jumped { by: 10_000_001 }));
        assert_eq!(
            step(-max - 1, 0),
            Err(ClockFault::Jumped { by: -10_000_001 })
        );
        assert_eq!(step(i128::MAX, 0), Err(ClockFault::Jumped { by: i64::MAX }));
        assert_eq!(
            step(i128::MIN + 1, 0),
            Err(ClockFault::Jumped { by: i64::MIN })
        );
    }

    #[test]
    fn uncertain_moments_excuse_a_step_and_widen_the_bound() {
        let max = i128::from(MAX_STEP);
        assert_eq!(step(max + 3, 3), Ok(MAX_STEP + 6));
        assert_eq!(step(max + 4, 3), Err(ClockFault::Jumped { by: 10_000_004 }));
        assert_eq!(step(0, 3), Ok(3));
        assert_eq!(step(0, u64::MAX), Ok(u64::MAX));
    }

    #[test]
    fn a_wait_lasts_until_settled_is_one_past() {
        let wait = |settled, revision| until_past(Revision(settled), Revision(revision));
        assert_eq!(wait(10, 14), Some(Duration::from_nanos(5)));
        assert_eq!(wait(14, 14), Some(Duration::from_nanos(1)));
        assert_eq!(wait(15, 14), None);
    }

    #[test]
    fn a_sample_reads_off_the_monotonic_clock_and_widens_with_drift() {
        let at = |wall, monotonic, uncertainty| Moment {
            wall,
            monotonic,
            uncertainty,
        };
        let taken = Taken {
            sample: sample(100, 7),
            at: at(1_000, 500, 2),
        };
        // 10 µs on both clocks: drift adds 5 ns, and the two moments' 2 ns
        // and 3 ns of uncertainty add 5 more.
        assert_eq!(taken.range(at(11_000, 10_500, 3)), Ok((11_083, 11_117)));
        // The system clock stepped 1 ms ahead: true time still follows the
        // monotonic clock, and the step widens the bound.
        assert_eq!(
            taken.range(at(1_011_000, 10_500, 3)),
            Ok((11_083 - 1_000_000, 11_117 + 1_000_000))
        );
        assert_eq!(
            taken.range(at(1_000 + 11 * MS, 500, 0)),
            Err(ClockFault::Jumped { by: 11_000_000 })
        );
        // A moment from before the sample counts no time as elapsed.
        assert_eq!(taken.range(at(1_000, 0, 0)), Ok((1_091, 1_109)));
    }

    #[test]
    fn ranges_meet_where_they_all_overlap() {
        assert_eq!(meet([(0, 10), (5, 15), (-3, 8)]), Some((5, 8)));
        assert_eq!(meet([(0, 5), (5, 9)]), Some((5, 5)));
        assert_eq!(meet([(0, 4), (5, 9)]), None);
        assert_eq!(meet([]), None);
    }

    #[test]
    fn a_window_reads_where_its_samples_meet() {
        let sampled = Sampled::new(sample(0, 10 * MS), OsTime);
        assert!((20 * MS..21 * MS).contains(&width(sampled.now().unwrap())));
        sampled.add(sample(5 * MS as i64, 10 * MS)).unwrap();
        // [-10, 10] and [-5, 15] meet in [-5, 10].
        let now = sampled.now().unwrap();
        assert!((15 * MS..16 * MS).contains(&width(now)));
        assert!(now.settled.0.abs_diff(wall_nanos()) < 10 * MS);
    }

    #[test]
    fn a_sample_that_disagrees_resets_the_window() {
        let sampled = Sampled::new(sample(0, MS), OsTime);
        assert_eq!(
            sampled.add(sample(1_000 * MS as i64, MS)),
            Err(ClockFault::Drifted)
        );
        let now = sampled.now().unwrap();
        let ahead = wall_nanos() + 1_000 * MS;
        assert!(now.settled.0.abs_diff(ahead) < 10 * MS, "{now:?}");
        assert!(width(now) < 3 * MS);
    }

    #[test]
    fn a_window_holds_the_last_eight_samples() {
        let sampled = Sampled::new(sample(0, MS), OsTime);
        for _ in 0..7 {
            sampled.add(sample(0, 100 * MS)).unwrap();
        }
        assert!(width(sampled.now().unwrap()) < 3 * MS);
        sampled.add(sample(0, 100 * MS)).unwrap();
        assert!(width(sampled.now().unwrap()) >= 200 * MS);
    }

    #[test]
    fn a_window_too_wide_is_unsynchronized() {
        let sampled = Sampled::new(sample(0, 2 * MAX_BOUND), OsTime);
        assert_eq!(sampled.now(), Err(ClockFault::Unsynchronized));
    }
}
