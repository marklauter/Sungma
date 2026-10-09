---
title: Clock work left
type: note
summary: The clock in docs/specs/clock.md is built; what remains is deferred until Sungma runs on EC2, unverified on Fargate, or waits on the merge.
status: evolving
---

# Clock work left

## Where it stands

The clock that [[clock]] specifies is built in `sungma` (the port) and `sungma-clock` (the clocks), on the `crate-split` branch. What follows is outside the spec: work deferred, questions not yet answered, and checks that run after the merge.

## Deferred

- **A ClockBound clock.** AWS ClockBound publishes a bound of microseconds, which would cut commit-wait from tens of milliseconds. It runs as a daemon on the host, and ECS on Fargate has no host to run it on, so it waits for an EC2 deployment. It would be its own `SystemClock` variant, reading ClockBound's shared memory through `clock-bound-client` 2.0.3 behind an opt-in feature; ClockBound 3.0 was still in beta when this was decided.
- **A soak test.** A long run comparing readings against a reference clock, such as ClockBound or a PTP hardware clock, counting readings whose range misses it. Neither CI nor Fargate has a reference clock, so it waits for EC2 too.
- **NTS from ntp-proto.** ntp-proto is the protocol crate under ntpd-rs, an audited NTP daemon in Rust from the Trifecta Tech Foundation. Its NTS key exchange, AES-SIV extension fields, cookies, NAK check and packet parsing could replace `nts.rs` and the packet code in `ntp.rs`, so the authenticated protocol no longer rests on our own review. The deadlines, size caps, address shares, quorum and Kiss-o'-Death limits stay ours, as do the window and bounds. Before adopting it: whether those parts can be used apart from the daemon, how stable its API is for outside callers, and its audit's scope.

Both the ClockBound clock and the soak test are also in `docs/roadmap.md` under Deferred.

## Unverified on Fargate

- Whether a Fargate task's kernel reports a synchronized clock. If it does, `SystemClock` takes `LinuxClock`, with a bound from the platform's own time sync.
- Whether a task can reach Amazon Time Sync at `169.254.169.123`. It smears leap seconds, so it can't join the public NTS servers in a quorum either way.

## After the merge

- Dispatch the `Clock platforms` workflow once: `LinuxClock` under chrony, and NTS from the public servers on Linux and Windows. It then runs weekly.
- Run `cargo bench -p sungma-clock` on Linux for `linux_now` at 32 threads. `LinuxClock` reads the kernel under its lock, and the cost under contention hasn't been measured.

## Checking Linux on Windows

`scripts/linux.sh` runs any cargo command for the Linux target in a podman container: clippy, the tests, `cargo mutants` on `linux.rs` and `time.rs`, and the benches. The container shares the kernel of podman's virtual machine, so `adjtimex` reads that machine's clock, which the container can't discipline. The weekly workflow stays the test of `LinuxClock` under chrony.
