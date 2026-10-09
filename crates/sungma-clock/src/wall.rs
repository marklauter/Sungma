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
    since_epoch(SystemTime::now())
}

/// Nanoseconds from the Unix epoch to `time`, or 0 for a time before it.
fn since_epoch(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
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
/// [`MAX_STEP`] by more than the slack and `excused`, the most the two
/// clocks may run apart without a step, as when a time service slews the
/// system clock but not the monotonic one.
pub(crate) fn step(step: i128, slack: u64, excused: u64) -> Result<u64, ClockFault> {
    let size = step.unsigned_abs();
    if size > u128::from(MAX_STEP) + u128::from(slack) + u128::from(excused) {
        let by = step.clamp(i64::MIN.into(), i64::MAX.into()) as i64;
        return Err(ClockFault::Jumped { by });
    }
    Ok(saturate(size).saturating_add(slack))
}

/// Nanoseconds of `elapsed`, signed.
pub(crate) fn nanos(elapsed: Duration) -> i128 {
    i128::from(saturate(elapsed.as_nanos()))
}

/// How long until a clock whose settled time is `settled`, at or before
/// `revision`, passes it.
pub(crate) fn until_past(settled: Revision, revision: Revision) -> Duration {
    Duration::from_nanos(revision.0.saturating_sub(settled.0).saturating_add(1))
}

/// Sleeps until `clock`'s settled time is past `stamped.revision`. A bound
/// that grows during a sleep takes another, and a fault ends the wait.
pub(crate) async fn wait(clock: &(impl Clock + Sync), stamped: Reading) -> Result<(), ClockFault> {
    loop {
        let settled = clock.now()?.settled;
        if settled > stamped.revision {
            return Ok(());
        }
        tokio::time::sleep(until_past(settled, stamped.revision)).await;
    }
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
        // Off Linux the monotonic clock isn't slewed with the system clock,
        // so the two may run apart by the drift allowance either way.
        let excused = drift(elapsed).saturating_mul(2);
        let widen = step(moved - elapsed_nanos, slack, excused)?;
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

    /// Monotonic time since the newest sample was taken.
    pub(crate) fn since_newest(&self) -> Duration {
        let window = self.window.lock().unwrap_or_else(PoisonError::into_inner);
        let newest = window.back().expect("a window always holds a sample");
        Duration::from_nanos(self.time.monotonic().saturating_sub(newest.at.monotonic))
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
    fn a_system_clock_before_1970_reads_as_zero() {
        assert_eq!(since_epoch(UNIX_EPOCH - Duration::from_secs(1)), 0);
        // Windows keeps system time in 100 ns ticks.
        assert_eq!(since_epoch(UNIX_EPOCH + Duration::from_nanos(700)), 700);
    }

    /// A clock that reads from a script, one result per reading, and then
    /// keeps the last.
    struct Scripted(Mutex<Vec<Result<Reading, ClockFault>>>);

    impl Scripted {
        fn new(readings: &[Result<(u64, u64), ClockFault>]) -> Self {
            let readings = readings.iter().rev().map(|reading| {
                reading.map(|(settled, revision)| Reading {
                    settled: Revision(settled),
                    revision: Revision(revision),
                })
            });
            Self(Mutex::new(readings.collect()))
        }

        fn left(&self) -> usize {
            self.0.lock().unwrap().len()
        }
    }

    impl Clock for Scripted {
        fn now(&self) -> Result<Reading, ClockFault> {
            let mut script = self.0.lock().unwrap();
            if script.len() > 1 {
                script.pop().unwrap()
            } else {
                script[0]
            }
        }

        fn wait(&self, stamped: Reading) -> impl Future<Output = Result<(), ClockFault>> + Send {
            wait(self, stamped)
        }
    }

    fn stamped(revision: u64) -> Reading {
        Reading {
            settled: Revision(0),
            revision: Revision(revision),
        }
    }

    /// Waits on a stamp at `revision`, failing a wait that doesn't end.
    async fn waited(clock: &Scripted, revision: u64) -> Result<(), ClockFault> {
        let wait = clock.wait(stamped(revision));
        tokio::time::timeout(Duration::from_secs(1), wait)
            .await
            .expect("the wait ends")
    }

    #[tokio::test]
    async fn a_wait_sleeps_again_when_the_bound_grows_during_a_sleep() {
        let clock = Scripted::new(&[Ok((10, 20)), Ok((12, 30)), Ok((15, 40)), Ok((99, 99))]);
        assert_eq!(waited(&clock, 14).await, Ok(()));
        assert_eq!(clock.left(), 1);
        assert_eq!(clock.now().unwrap().settled, Revision(99));
    }

    #[tokio::test]
    async fn a_wait_on_a_stamp_equal_to_settled_sleeps() {
        let clock = Scripted::new(&[Ok((14, 20)), Ok((15, 20)), Ok((0, 0))]);
        assert_eq!(waited(&clock, 14).await, Ok(()));
        assert_eq!(clock.left(), 1);
    }

    #[tokio::test]
    async fn a_wait_on_a_passed_stamp_reads_once() {
        let clock = Scripted::new(&[Ok((15, 20)), Ok((0, 0))]);
        assert_eq!(waited(&clock, 14).await, Ok(()));
        assert_eq!(clock.left(), 1);
    }

    #[tokio::test]
    async fn a_fault_during_a_wait_ends_it() {
        let clock = Scripted::new(&[Ok((10, 20)), Err(ClockFault::Drifted)]);
        assert_eq!(waited(&clock, 14).await, Err(ClockFault::Drifted));
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
        assert_eq!(step(max, 0, 0), Ok(MAX_STEP));
        assert_eq!(step(-max, 0, 0), Ok(MAX_STEP));
        assert_eq!(
            step(max + 1, 0, 0),
            Err(ClockFault::Jumped { by: 10_000_001 })
        );
        assert_eq!(
            step(-max - 1, 0, 0),
            Err(ClockFault::Jumped { by: -10_000_001 })
        );
        assert_eq!(
            step(i128::MAX, 0, 0),
            Err(ClockFault::Jumped { by: i64::MAX })
        );
        assert_eq!(
            step(i128::MIN + 1, 0, 0),
            Err(ClockFault::Jumped { by: i64::MIN })
        );
    }

    #[test]
    fn uncertain_moments_excuse_a_step_and_widen_the_bound() {
        let max = i128::from(MAX_STEP);
        assert_eq!(step(max + 3, 3, 0), Ok(MAX_STEP + 6));
        assert_eq!(
            step(max + 4, 3, 0),
            Err(ClockFault::Jumped { by: 10_000_004 })
        );
        assert_eq!(step(0, 3, 0), Ok(3));
        assert_eq!(step(0, u64::MAX, 0), Ok(u64::MAX));
    }

    #[test]
    fn a_divergence_within_the_drift_allowance_isnt_a_jump() {
        let max = i128::from(MAX_STEP);
        // Excused, but still widening the bound by its whole size.
        assert_eq!(step(max + 5, 0, 5), Ok(MAX_STEP + 5));
        assert_eq!(
            step(max + 6, 0, 5),
            Err(ClockFault::Jumped { by: 10_000_006 })
        );
        assert_eq!(
            step(max + 9, 3, 5),
            Err(ClockFault::Jumped { by: 10_000_009 })
        );
        assert_eq!(step(max + 8, 3, 5), Ok(MAX_STEP + 11));
    }

    #[test]
    fn a_wait_lasts_until_settled_is_one_past() {
        let wait = |settled, revision| until_past(Revision(settled), Revision(revision));
        assert_eq!(wait(10, 14), Duration::from_nanos(5));
        assert_eq!(wait(14, 14), Duration::from_nanos(1));
        assert_eq!(wait(0, u64::MAX), Duration::from_nanos(u64::MAX));
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
    fn a_system_clock_slewed_against_the_monotonic_one_isnt_a_jump() {
        let taken = Taken {
            sample: sample(0, 0),
            at: Moment {
                wall: 0,
                monotonic: 0,
                uncertainty: 0,
            },
        };
        let after = |wall| Moment {
            wall,
            monotonic: 100 * 1_000 * MS,
            uncertainty: 0,
        };
        // A hundred seconds, with the system clock slewed 40 ms ahead of the
        // monotonic clock: drift allows 50 ms each way, so 100 ms apart.
        let slewed = 100 * 1_000 * MS + 40 * MS;
        let drifted = i128::from(drift(Duration::from_secs(100)));
        let (low, high) = taken.range(after(slewed)).unwrap();
        assert_eq!(high - low, 2 * (drifted + i128::from(40 * MS)));
        let past = 100 * 1_000 * MS + 110 * MS + 1;
        assert!(matches!(
            taken.range(after(past)),
            Err(ClockFault::Jumped { .. })
        ));
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

/// The clocks against a model of true time: nodes whose clocks drift and
/// step within what a sample allows, and whose samples are honest.
#[cfg(test)]
mod simulation {
    use std::sync::Arc;

    use proptest::{collection::vec, prelude::*, test_runner::RngSeed};

    use super::*;
    use crate::ManualTime;

    const MS: i64 = 1_000_000;

    /// A fixed seed, so a failure reproduces from the seed proptest prints.
    fn config() -> ProptestConfig {
        ProptestConfig {
            rng_seed: RngSeed::Fixed(20_261_008),
            // A broken clock fails fast instead of shrinking for minutes.
            max_shrink_iters: 64,
            ..ProptestConfig::default()
        }
    }

    /// 2026-10-08T00:00:00Z.
    const START: i128 = 1_791_417_600_000_000_000;

    /// How far a node's monotonic clock may run from true time over
    /// `elapsed`, kept just inside the drift allowance.
    fn honest_drift(elapsed: u64) -> u64 {
        elapsed * 499 / 1_000_000
    }

    /// A node: its clocks, its sampling clock, and its drift, from -1 to 1
    /// of the allowance.
    struct Node {
        time: Arc<ManualTime>,
        clock: Sampled<Arc<ManualTime>>,
        drift: f64,
    }

    impl Node {
        /// A node whose system clock is `offset` behind true time `now`,
        /// sampled with an error of `error` (from -1 to 1) of `bound`.
        fn new(now: i128, offset: i64, bound: u64, error: f64, drift: f64) -> Self {
            let wall = u64::try_from(now - i128::from(offset)).unwrap();
            let time = Arc::new(ManualTime::new(wall));
            let clock = Sampled::new(sample(offset, bound, error), time.clone());
            Self { time, clock, drift }
        }

        /// True time passes `by` nanoseconds; this node's clocks count
        /// that, give or take its drift.
        fn pass(&self, by: u64) {
            let drift = (self.drift * honest_drift(by) as f64) as i64;
            self.time
                .advance(Duration::from_nanos(by.saturating_add_signed(drift)));
        }

        /// How far behind true time `now` the system clock is.
        fn offset(&self, now: i128) -> i64 {
            i64::try_from(now - i128::from(self.time.wall())).unwrap()
        }
    }

    fn sample(offset: i64, bound: u64, error: f64) -> Sample {
        Sample {
            offset: offset + (error * bound as f64) as i64,
            bound,
        }
    }

    #[derive(Clone, Debug)]
    enum Event {
        /// True time passes this many milliseconds.
        Pass(u64),
        /// The system clock steps this many nanoseconds.
        Step(i64),
        /// A fresh sample, with this error and bound.
        Resync(f64, u64),
    }

    fn events() -> impl Strategy<Value = Vec<Event>> {
        let event = prop_oneof![
            (0u64..600_000).prop_map(Event::Pass),
            (-12 * MS..12 * MS).prop_map(Event::Step),
            (-1.0..1.0, 1_000u64..100_000_000).prop_map(|(e, b)| Event::Resync(e, b)),
        ];
        vec(event, 1..30)
    }

    proptest! {
        #![proptest_config(config())]

        /// Every reading contains true time, or the clock reports why it
        /// can't: a step past 10 ms since its newest sample, or a bound
        /// past a second. Honest samples never disagree.
        #[test]
        fn a_reading_contains_true_time(
            offset in -10_000 * MS..10_000 * MS,
            bound in 1_000u64..100_000_000,
            error in -1.0..1.0f64,
            drift in -1.0..1.0f64,
            events in events(),
        ) {
            let mut now = START;
            let node = Node::new(now, offset, bound, error, drift);
            let mut stepped = 0i64;
            // The newest sample's bound, and the monotonic time it was
            // taken at. The reading is no wider than this sample allows.
            let mut newest = (bound, node.time.monotonic());
            for event in events {
                match event {
                    Event::Pass(ms) => {
                        node.pass(ms * 1_000_000);
                        now += i128::from(ms) * 1_000_000;
                    }
                    Event::Step(by) => {
                        node.time.step(by);
                        stepped += by;
                    }
                    Event::Resync(error, bound) => {
                        let fresh = sample(node.offset(now), bound, error);
                        prop_assert_eq!(node.clock.add(fresh), Ok(()));
                        stepped = 0;
                        newest = (bound, node.time.monotonic());
                    }
                }
                // The newest sample's bound now: drift since it at 500 ppm,
                // plus the steps since it. Narrower samples only narrow it.
                let elapsed = node.time.monotonic() - newest.1;
                let allowed = newest.0 + elapsed * 500 / 1_000_000 + stepped.unsigned_abs();
                // A step faults only past 10 ms plus twice the drift allowance.
                let jumps = MAX_STEP + 2 * (elapsed * 500 / 1_000_000);
                match node.clock.now() {
                    Ok(reading) => {
                        prop_assert!(stepped.unsigned_abs() <= jumps);
                        let width = reading.revision.0 - reading.settled.0;
                        prop_assert!(width <= 2 * allowed, "{width} > 2 * {allowed}");
                        let (settled, revision) = (reading.settled.0, reading.revision.0);
                        prop_assert!(i128::from(settled) <= now && now <= i128::from(revision));
                    }
                    Err(ClockFault::Jumped { by }) => {
                        prop_assert!(stepped.unsigned_abs() > jumps);
                        prop_assert_eq!(by, stepped);
                    }
                    Err(ClockFault::Unsynchronized) => {
                        prop_assert!(stepped.unsigned_abs() <= jumps);
                        prop_assert!(allowed > MAX_BOUND, "{allowed} is within a second");
                    }
                    Err(other) => prop_assert!(false, "{other:?}"),
                }
            }
        }

        /// Commit-wait across nodes, with writes that overlap: each write
        /// stamps its node's revision when it starts, and is acknowledged
        /// once that node's settled time passes it. For every pair where
        /// one write starts after the other is acknowledged, the later
        /// write has the greater revision, whichever nodes they run on.
        #[test]
        fn a_later_write_gets_a_later_revision(
            nodes in vec(
                (-10_000 * MS..10_000 * MS, 1_000u64..50_000_000, -1.0..1.0f64, -1.0..1.0f64),
                2..5,
            ),
            history in vec(prop_oneof![
                (0usize..5).prop_map(Some),
                Just(None),
            ], 2..40),
        ) {
            let mut now = START;
            let nodes: Vec<_> = nodes
                .into_iter()
                .map(|(offset, bound, error, drift)| Node::new(now, offset, bound, error, drift))
                .collect();
            // Each write: its node, revision, start, and acknowledgement.
            let mut writes: Vec<(usize, Revision, i128, Option<i128>)> = Vec::new();
            let settle = |now: i128, writes: &mut Vec<(usize, Revision, i128, Option<i128>)>| {
                for (node, revision, _, acknowledged) in writes.iter_mut() {
                    if acknowledged.is_none() && nodes[*node].clock.now().unwrap().settled > *revision {
                        *acknowledged = Some(now);
                    }
                }
            };
            for event in history.into_iter().map(Some).chain(std::iter::repeat_n(None, 200)) {
                match event {
                    Some(Some(pick)) => {
                        let node = pick % nodes.len();
                        let revision = nodes[node].clock.now().unwrap().revision;
                        writes.push((node, revision, now, None));
                    }
                    // A millisecond of true time passes on every node.
                    Some(None) | None => {
                        for node in &nodes {
                            node.pass(1_000_000);
                        }
                        now += 1_000_000;
                    }
                }
                settle(now, &mut writes);
            }
            for (_, earlier, _, acknowledged) in &writes {
                let acknowledged = acknowledged.expect("every write is acknowledged");
                for (_, later, started, _) in &writes {
                    if *started >= acknowledged {
                        prop_assert!(later > earlier, "{later:?} started after {earlier:?}");
                    }
                }
            }
        }
    }

    /// The same order fails for a node whose sample claims less error than
    /// it has: its stamps can fall behind writes already acknowledged.
    #[test]
    fn a_node_outside_its_bound_can_stamp_an_earlier_revision() {
        let now = START;
        let honest = Node::new(now, 0, MS as u64, 0.0, 0.0);
        // 50 ms behind, sampled as if exact, with a 1 ms bound.
        let liar = Node::new(now, 50 * MS, MS as u64, -50.0, 0.0);
        let stamped = honest.clock.now().unwrap();
        for _ in 0..10 {
            honest.pass(1_000_000);
            liar.pass(1_000_000);
        }
        assert!(honest.clock.now().unwrap().settled > stamped.revision);
        let later = liar.clock.now().unwrap();
        assert!(later.revision < stamped.revision, "{later:?} {stamped:?}");
    }
}
