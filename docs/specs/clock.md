---
title: The clock
type: specification
summary: "A clock reading brackets true time between a settled time and a revision; a write is stamped with the revision and acknowledged once the clock's settled time has passed it."
status: evolving
cites:
  - "[[revision]]"
  - "[[revision-clock]]"
  - "[[revision-token]]"
  - "[[snapshot]]"
---

# The clock

A clock reads the [[revision-clock]]. It never knows true time, only a reading that brackets it, and a [[revision]] is taken from that reading. [[one-timeline]] orders facts and theories on these revisions.

## The port

`Clock` in `sungma::clock` has three methods:

- `now()` returns a `Reading { settled, revision }`, or a `ClockFault` when the clock can't be trusted. True time is certainly past `settled` and not yet past `revision`.
- `wait(stamped)` takes the reading a write was stamped with, and resolves once the clock's `settled` time is past `stamped.revision`, or with the fault that stopped the wait. The caller hands back the reading whole, so it can't compare the wrong fields.
- `observe(seen)` checks a revision another node stamped, such as one a [[revision-token]] carries. A revision past this clock's `revision` is one true time can't have reached yet, so one of the two clocks is wrong.

A write follows commit-wait:

1. Take `stamped = now()`.
2. Write at `stamped.revision`.
3. `wait(stamped)`.
4. Acknowledge, returning the [[revision-token]] for `stamped.revision`.

True time is not yet past the stamp, so a stamp is never in the past. After the wait, true time is past the stamp. A write that starts after the acknowledgement therefore stamps a later revision, on any node. A read at revision `t` waits until no write at or before `t` is still in flight, so a [[snapshot]] never shows a later write without an earlier one.

Revisions need not be unique. Two writes stamped with the same revision are concurrent, and a snapshot at that revision holds both.

The clock knows nothing of writes. The writer ports call it inside a commit, and the domain never does. Clocks that read time report nanoseconds since the Unix epoch.

## Faults

A clock refuses a reading rather than give one that may miss true time. The node then refuses to stamp until the clock recovers, and another node takes the write.

- `Jumped { by }`: the system clock moved more than 10 ms against the monotonic clock since the moment it was last compared at, as when a virtual machine is paused or moved, or a time service steps the clock. A sampling clock stays faulted until a fresh sample.
- `Drifted`: a new sample doesn't overlap the window, so the clock drifted faster than its allowance. The window restarts from the new sample.
- `Behind { seen, revision }`: `observe` was given a revision past this clock's `revision`.
- `Unsynchronized`: the platform reports its clock unsynchronized, or the bound grew past 1 s, the most a write should wait.

## Time sources

A clock reads two clocks from a `TimeSource`: the system clock, which a time service may step, and a monotonic clock, which never steps. `OsTime` is the operating system's. Its monotonic clock is `CLOCK_BOOTTIME` on Linux, which counts a suspend, and `Instant` elsewhere, where a suspend shows as a jump. `ManualTime`, behind the `test-util` feature, lets a test pass time on both clocks or step the system clock alone.

`moment()` reads the system clock between two readings of the monotonic clock, and takes the monotonic time as their middle. The gap between the two is the moment's uncertainty. It tries up to three times for a gap of 1 ms or less, and keeps the narrowest. A thread descheduled between the reads widens the reading by that uncertainty instead of reporting a jump.

After a sample, true time is read off the monotonic clock, so a step of the system clock doesn't move it. A step of up to 10 ms, plus the uncertainty of the two moments it was measured between, widens the bound by its size. A larger one is a jump.

## NTP

An NTP exchange records four times:

- T1: the client sends the request, by the client's clock.
- T2: the server receives it, by the server's clock.
- T3: the server sends the reply, by the server's clock.
- T4: the client receives the reply, by the client's clock.

From them:

- Offset = ((T2 − T1) + (T3 − T4)) / 2, how far the client's clock is behind the server's. It assumes the request and the reply took equal time.
- Round trip = (T4 − T1) − (T3 − T2), the time spent on the network.
- Bound = round trip / 2 + root delay / 2 + root dispersion, which is RFC 5905's root distance. The server read its clock at some point during the round trip, so the offset is off by at most half the round trip. The root delay and root dispersion are the server's own uncertainty about true time.

An exchange gives the range offset ± bound.

### Servers

The server list is injected into the synchronizer; the clock holds no list of its own. A deployment gives at least four servers from independent operators. A sync needs three answers, and the fourth keeps the sync going while one is unreachable. Servers that smear leap seconds, such as Google's and AWS's public servers, disagree with the rest near a leap second and aren't mixed with them.

Servers that offer NTS include `time.cloudflare.com`, `nts.netnod.se`, `ptbtime1.ptb.de` and `nts.time.nl`. `time.windows.com` doesn't.

### A sync

A sync asks every server at once. The clock keeps the range where the most servers agree, as Marzullo's algorithm does, and a server outside it is a falseticker. A sync needs answers from at least three servers, so the servers can outvote one wrong one; with fewer, it fails with `TooFewAnswers` and the window stays as it was. Of `n` answers, up to `(n − 1) / 2` falsetickers are tolerated; past that, the sync fails. A server that answers Kiss-o'-Death is asked less often, as it requests. A kiss over plain NTP isn't authenticated, so an attacker on the path can forge one: it holds a server for at most 1024 s, `DENY` and `RSTR` included, and a sync with fewer than three servers askable asks held servers anyway, those whose holds end soonest, and reports them as overridden. A kiss over NTS is honored as written. Server names are resolved again on each sync, so a pool's addresses can change. A server whose leap indicator announces a leap second is noted.

### The window

The clock keeps a window of the last eight samples. A sample's bound grows by the maximum drift, 500 ppm, from the moment it was taken, so an old sample stays valid but loosens. The clock intersects the window, so one exchange with a slow round trip doesn't widen the bound. A sample that doesn't overlap the window is a `Drifted` fault.

`now()` returns true time where the window's samples meet. It never touches the network. The system clock itself is never set.

### Resync

`resync()` takes a new sample into the window. When the servers can't be reached, the window stays as it was and its bounds keep growing. A refresher calls it every 64 seconds, NTP's shortest standard poll, and backs off when a server sends Kiss-o'-Death. At that rate drift adds at most 32 ms between syncs, 64 s at 500 ppm.

### NTS

Network Time Security, RFC 8915, authenticates NTP. A TLS 1.3 handshake on TCP port 4460 agrees keys and hands the client cookies. Each query, still over UDP port 123, then carries a cookie and an authentication tag, and a forged or altered reply is dropped. NTS doesn't prevent delay, but a delayed reply has a longer round trip, which the bound already counts. NTS replaces the transport; the offset, bound, sync and window stay as they are.

A key exchange finishes within 5 s in all, and its response is refused past 64 KiB. The client keeps at most eight cookies. A reply that doesn't verify is skipped, and the client waits for one that does until the query times out. A server forgets its cookies with an `NTSN` Kiss-o'-Death, which comes unauthenticated: the client honors it only when it echoes the request's unique identifier, and then repeats the key exchange.

`Servers::nts` names NTS servers, checked against Mozilla's roots, and `Servers::new` names plain NTP servers. A list can mix both, but a production list is all NTS.

## Clocks

- `SystemClock` is the production clock. At startup it takes the Linux kernel's clock when the kernel reports a bound, and otherwise an `NtpClock`. It is an enum, not a trait object, because `wait` returns `impl Future`.
- `LinuxClock` reads the kernel's `maxerror` through `adjtimex`. chrony keeps it current, and the kernel grows it by 500 ppm between updates. An unsynchronized kernel faults, since the kernel stops growing `maxerror` at 16 s. Each reading compares the system clock with the monotonic clock since the last, to catch a step the kernel's bound doesn't cover.
- `NtpClock` asks the servers directly, and is the only clock on Windows.
- `DevClock` is a counter with `settled` equal to `revision`, for a single node. Its waits never sleep.
- `ManualClock`, behind the `test-util` feature, is set by hand to a reading or a fault. A test can hold a write in its commit-wait and then release it.

## Status

`status()` on `NtpClock`, `LinuxClock` and `SystemClock` gives a `ClockStatus` for metrics: the reading's width, the current fault if any, counts of each fault kind since start, the time since the newest sample, the failed syncs, and the last sync's report. It doesn't take a reading, so it never counts a fault or moves `LinuxClock`'s jump check. Exporting it, and alerting on it, belong to the API's operations layer. The refresher's report callback carries each refresh's result.

## Deferred

- **A ClockBound clock**, reading AWS ClockBound's earliest and latest times. ClockBound runs as a daemon on the host, and ECS on Fargate, Sungma's likely home, has no host to run it on. It returns as its own `SystemClock` variant when Sungma runs on EC2.
- **A soak test** against a reference clock, such as ClockBound or a PTP hardware clock, counting the readings whose range misses the reference. Neither CI nor Fargate has a reference clock. It runs when Sungma runs on EC2.

## Deployment

On ECS on Fargate, `SystemClock` takes `NtpClock` unless the task's kernel reports a synchronized clock, which is unverified.

- A task reaches at least three servers from independent operators: outbound UDP 123 for queries and TCP 4460 for NTS key exchange, through a NAT gateway or a public IP.
- Internet servers give bounds of tens of milliseconds, and the bound grows by up to 32 ms between syncs, so commit-wait adds tens of milliseconds to each write.
- Amazon Time Sync at `169.254.169.123` smears leap seconds, so it isn't mixed with the servers above. Whether a Fargate task can reach it is unverified.

## Required tests

A clock whose reading misses true time reverses revisions without an error. A check can then miss a revocation. Its tests therefore cover a clock that is wrong, stepped, paused or lied to, as well as one in good order. Each class below is required.

1. **Readings.** Every reading has `settled` at or before `revision`, contains true time, and saturates rather than wraps at both ends of `u64`. A bound past 1 s faults.
2. **Commit-wait.** `wait` resolves only once `settled` is past the stamp, takes another sleep when the bound grows during one, returns at once when the stamp is already past, and ends with a fault the clock reports.
3. **Real-time order.** Across simulated nodes whose clocks err anywhere within their bounds, a write that starts after another is acknowledged gets a later revision.
4. **Steps and pauses.** A step within the allowance widens the reading, a larger one faults, a suspend on Linux still reads true time, and a thread descheduled between clock reads widens the reading rather than faulting.
5. **Drift and the window.** A sample's bound grows by 500 ppm of the time since it was taken. The window reads where its samples meet, holds eight, and restarts on a sample that disagrees. A failed resync keeps the window.
6. **Faults.** Each fault is reported where [Faults](#faults) says, and `observe` faults on a revision past the clock's.
7. **NTP arithmetic.** Timestamps convert between NTP and Unix time to the nanosecond in both eras, and offset, round trip and root distance follow [NTP](#ntp).
8. **NTP replies.** Each refusal: wrong length, not a server's reply, an unsynchronized server, and a reply to another request. A server's clock that runs backwards gives no negative round trip.
9. **Agreement.** The majority's range is kept, falsetickers up to the tolerance are dropped, and more fail the sync, as do fewer than three answers.
10. **The synchronizer.** Parallel queries, Kiss-o'-Death, re-resolution, the leap indicator, and the refresher's interval and backoff.
11. **NTS.** Key exchange, cookies, and refusal of a forged or altered reply.
12. **Platform bounds.** Linux's bound is the kernel's `maxerror`, and an unsynchronized kernel faults.
13. **Choosing the clock.** `SystemClock` takes the Linux kernel's clock when it reports a bound and NTP otherwise, and `refresh` takes a new sample for a clock that samples.
14. **Test clocks.** `DevClock` counts one past its last reading, never waits, and starts after a given revision. `ManualClock` holds a wait until it is set past the stamp, and a fault ends it.
15. **Platforms.** A workflow, run weekly and by hand, runs the clocks against real time services: `LinuxClock` under chrony, and NTS from the public servers on Linux and Windows.
16. **Fuzzing.** A `cargo-fuzz` target, and a property test in CI, assert the NTP reply parser never panics on arbitrary input.
17. **Mutation testing.** `cargo mutants` leaves no surviving mutant in the clock arithmetic.

Classes 3 and 4 drive the clocks through `ManualTime`. Commit-wait in a writer, and a read waiting for writes in flight, are tested with the writer ports.

### Cases

The cases each class covers, at minimum.

#### Readings

1. A property test over samples, moments and elapsed times asserts `settled <= revision`, and that a modeled true time within the sample's bound plus drift lies between them.
2. A reading near zero saturates `settled` at 0, and one near `u64::MAX` saturates `revision` at `u64::MAX`.
3. A bound of exactly 1 s gives a reading, and one past it faults `Unsynchronized`.

#### Commit-wait

1. `wait` on a stamp whose revision equals `settled` sleeps at least 1 ns.
2. `wait` on a stamp already passed resolves without sleeping.
3. With a test clock whose bound grows during a sleep, `wait` sleeps again and resolves only once `settled` is past the stamp.
4. After `wait` resolves, `now().settled` is past the stamp, for each clock.
5. A fault set while a wait sleeps ends the wait with that fault.

#### Real-time order

1. A simulation runs several nodes, each with a clock offset anywhere within its bound and drifting at up to 500 ppm, and random histories of overlapping and sequential writes. For every pair where one write starts after the other is acknowledged, the later write's revision is greater.
2. The same simulation with one clock outside its bound finds a reversed pair.
3. The simulation runs on paused time with a fixed seed, and a failure prints the seed.

#### Steps and pauses

1. An `NtpClock` on `ManualTime`, stepped 5 ms forward or back, reads true time with its bound widened by 5 ms.
2. Stepped 20 ms, it faults `Jumped` with the step's size and sign, and reads again after a fresh sample.
3. A `LinuxClock` on `ManualTime` faults on a 20 ms step, and reads again at the next moment.
4. On Linux, time passed on `CLOCK_BOOTTIME` and the system clock together, as in a suspend, gives a reading that contains true time.
5. A moment whose monotonic reads are far apart is retried up to three times, keeps the narrowest, and excuses that much step.
6. A system clock before the Unix epoch reads as 0 without a panic.

#### Drift and the window

1. A sample's bound after 2 s has grown by 1 ms.
2. `drift(Duration::MAX)` saturates at `u64::MAX`.
3. Two samples of ±10 ms, 5 ms apart, read as their 15 ms overlap.
4. Eight wide samples after a narrow one push the narrow one out of the window.
5. A sample 1 s from the window faults `Drifted`, and the clock then reads from it alone.
6. After a failed resync, the reading's width is the window's bounds plus drift since each sample, not since the failure.

#### Faults

1. `observe` on a revision at or below the clock's `revision` passes, and one past it faults `Behind` with both revisions.
2. `observe` on a faulted clock returns the clock's fault.
3. A window whose bound is past 1 s faults `Unsynchronized`.

#### NTP arithmetic

1. A property test asserts Unix to NTP to Unix is within 1 ns, for times across both eras.
2. Times on both sides of 2036-02-07T06:28:16Z, when NTP's seconds wrap, read back in the right era.
3. A 16.16 short of `0x0001_0000` is 1 s.
4. A reply from a server 5 s ahead, 10 ms out, 30 ms back and held 2 ms, gives an offset of 4,990 ms and a round trip of 40 ms.

#### NTP replies

1. A reply of 47 bytes, a client packet, a leap indicator of 3, stratum 0 and stratum 16 are each refused with their reason. Stratum 15 is accepted.
2. A reply whose origin isn't the request's transmit time is refused.
3. A reply whose offset doesn't fit an `i64` is refused.
4. A server that doesn't answer within the timeout is skipped, and a sync where none answers fails.
5. A request is a version 4 client packet carrying its send time, and nothing else.
6. Over IPv4 and IPv6 loopback, a simulated server's time is read within its bound.

#### Agreement

1. Four servers, one 100 s off with a small bound: the sync keeps the other three's range and names the falseticker.
2. Four servers, two of them wrong in different directions: the sync fails.
3. Ranges that touch at one point give a bound of 0, and an odd-width range rounds the bound up.
4. A property test asserts the kept range lies inside every agreeing server's range and doesn't depend on their order.
5. Four servers with two unreachable: two answers that agree still fail the sync, and the window stays as it was.

#### The synchronizer

1. A sync with one server that never answers finishes within one timeout, not one per server.
2. A Kiss-o'-Death `RATE` reply doubles that server's interval, and it isn't asked again sooner, but never holds it past 1024 s.
3. A `DENY` server is asked again after 1024 s.
4. Three servers all kissed `DENY`: the next sync asks all three and reports them as overridden. Four servers with one kissed `RATE`: it is held, not overridden.
5. A server name that resolves to a new address on the next sync is asked at the new address.
6. The refresher calls `resync` every 64 s on paused time, and a failed resync leaves the window and the schedule as they were.

#### NTS

1. Against a local NTS server, key exchange gives cookies, and a query with one is answered and authenticated.
2. A reply with an altered byte, or a wrong tag, is refused.
3. A cookie is used once, and a fresh one from each reply replaces it.

#### Platform bounds

1. Linux: `TIME_OK` is synchronized, and `-1`, `TIME_ERROR` and `STA_UNSYNC` are not.
2. Linux: `maxerror` in microseconds becomes the bound in nanoseconds, and an unreadable or negative `maxerror` is the 16 s phase limit.
3. Linux: `STA_UNSYNC` faults `Unsynchronized`, so a reading never claims the kernel's capped 16 s bound.

#### Choosing the clock

1. `SystemClock::detect` with a simulated server reads true time and waits it out on every platform.
2. A `SystemClock` over NTP takes a new sample on `refresh`.

#### Test clocks

1. `DevClock` reads 1, then 2. Started after 41, it reads 42.
2. `DevClock`'s `wait` on 9 moves its next reading to 10, and a later `wait` on 3 doesn't move it back.
3. `ManualClock` holds a wait while `settled` is at or before the stamp, and releases it once `settled` passes.
4. A fault set on `ManualClock` ends a wait and every reading until it is set again.

#### Platforms

1. Linux, with chrony synchronized, detects a `LinuxClock` whose reading contains the system time, and prints the kernel's `maxerror`. The fallback when chrony stops is covered by the unit tests on the kernel's status.
2. Linux and Windows sync an `NtpClock` from the public NTS servers and read within its bound.

#### Fuzzing

1. A `cargo-fuzz` target runs the NTP reply parser on arbitrary bytes and send and receive times.
2. A property test runs it in CI. It never panics or aborts.

#### Mutation testing

1. `cargo mutants` leaves no surviving mutant in `sungma-clock`'s `wall.rs`, `time.rs`, `ntp.rs` and `linux.rs`.

## References

- Keith Marzullo. *Maintaining the Time in a Distributed System.* PhD thesis, Stanford University, 1984. Intersecting intervals from several time sources, and rejecting the ones that don't agree.
- David L. Mills, Jim Martin, Jack Burbank and William Kasch. *Network Time Protocol Version 4: Protocol and Algorithms Specification.* RFC 5905, IETF, 2010. The four timestamps, root distance, and the clock filter's window of samples.
- Daniel Franke, Dieter Sibold, Kristof Teichel, Marcus Dansarie and Ragnar Sundblad. *Network Time Security for the Network Time Protocol.* RFC 8915, IETF, 2020. NTS key exchange, cookies and authenticated NTP.
- James C. Corbett et al. "Spanner: Google's Globally-Distributed Database." *OSDI*, 2012. TrueTime's interval, and commit-wait.
- chrony, <https://chrony-project.org>. An NTP implementation that measures the clock's drift rate and keeps the kernel's error bound current.
- AWS ClockBound, <https://github.com/aws/clock-bound>. A daemon that publishes a clock's bound as an earliest and a latest time, the same shape as a reading.
