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

`Clock` in `sungma::clock` has two methods:

- `now()` returns a `Reading { settled, revision }`. True time is certainly past `settled` and not yet past `revision`.
- `wait(stamped)` takes the reading a write was stamped with, and resolves once the clock's `settled` time is past `stamped.revision`. The caller hands back the reading whole, so it can't compare the wrong fields.

A write follows commit-wait:

1. Take `stamped = now()` while holding the write's locks.
2. Write at `stamped.revision`.
3. `wait(stamped)`.
4. Acknowledge, returning the [[revision-token]] for `stamped.revision`.

True time is not yet past the stamp, so a stamp is never in the past. After the wait, true time is past the stamp. A write that starts after the acknowledgement therefore stamps a later revision, on any node. A read at revision `t` waits until no write at or before `t` is still in flight, so a [[snapshot]] never shows a later write without an earlier one.

The writer ports call the clock inside a commit, and the domain never does. Clocks that read time report nanoseconds since the Unix epoch.

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

An exchange gives the range offset ± bound. Every server asked must overlap: the clock takes the intersection of their ranges, and when two ranges don't meet, the sync fails. A server with a wrong time and a small bound fails the sync rather than shifting the offset.

The clock keeps a window of recent samples. A sample's bound grows by the maximum drift, 500 ppm, from the moment it was taken, so an old sample stays valid but loosens. The clock intersects the window, so one exchange with a slow round trip doesn't widen the bound. The window isn't built yet: `NtpClock` keeps only its latest sample.

`now()` returns the system clock plus the offset, give or take the bound. The system clock itself is never set. `resync()` takes a new sample.

## Clocks

- `SystemClock` is the production clock. At startup it takes the platform's clock when the platform reports a bound, and otherwise an `NtpClock`. It is an enum, not a trait object, because `wait` returns `impl Future`.
- `LinuxClock` reads the kernel's `maxerror` through `adjtimex`. chrony keeps it current, and the kernel grows it by 500 ppm between updates.
- `WindowsClock` reads the Windows Time service's root delay and dispersion from `w32tm /query /status`, in English only. The service is often stopped, and its bound is often seconds wide, so Windows usually falls back to NTP.
- `NtpClock` asks servers such as `time.windows.com:123` directly. Servers that smear leap seconds, such as `time.google.com`, disagree with the rest near a leap second and shouldn't be mixed with them.
- `DevClock` is a counter with `settled` equal to `revision`, for a single node. Its waits never sleep.
- `ManualClock`, behind the `test-util` feature, is set by hand. A test can hold a write in its commit-wait and then release it.

## References

- Keith Marzullo. *Maintaining the Time in a Distributed System.* PhD thesis, Stanford University, 1984. Intersecting intervals from several time sources, and rejecting the ones that don't agree.
- David L. Mills, Jim Martin, Jack Burbank and William Kasch. *Network Time Protocol Version 4: Protocol and Algorithms Specification.* RFC 5905, IETF, 2010. The four timestamps, root distance, and the clock filter's window of samples.
- James C. Corbett et al. "Spanner: Google's Globally-Distributed Database." *OSDI*, 2012. TrueTime's interval, and commit-wait.
- chrony, <https://chrony-project.org>. An NTP implementation that measures the clock's drift rate and keeps the kernel's error bound current.
- AWS ClockBound, <https://github.com/aws/clock-bound>. A daemon that publishes a clock's bound as an earliest and a latest time, the same shape as a reading.
