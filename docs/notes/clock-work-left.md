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
Both the ClockBound clock and the soak test are also in `docs/roadmap.md` under Deferred.

## Explored and rejected

- **NTS from ntp-proto.** ntp-proto is the protocol crate under ntpd-rs, an audited NTP daemon in Rust from the Trifecta Tech Foundation. Its NTS key exchange, extension fields, cookies and packet parsing could have replaced `nts.rs` and the packet code in `ntp.rs`, so the authenticated protocol would rest on its review rather than ours. Checked against 1.9.0 in October 2026, it can't be used apart from the daemon:
  - Its NTS and packet types are exported only behind the `__internal-api` feature, and the crate says it is "not intended as a public interface" and gives no stability guarantee.
  - After a key exchange, the cookies and keys sit in `SourceNtsData`, whose fields are crate-private; the only accessors are compiled for tests or behind `__internal-test`. Building our own request would need a test feature in production, and the alternative, its `NtpSource`, brings its own polling and filtering in place of our quorum, Kiss-o'-Death limits and window.
  - Its key exchange trusts the platform's roots plus any extra ones, so `Servers::trusting_only`, which trusts a private root alone, couldn't be kept.
  - Its key exchange is async on tokio-rustls, with no deadline of its own.

  Other Rust NTS crates were no better reviewed than ours, or too young. NTP is the fallback behind `LinuxClock`, so `nts.rs` stays, its parsers fuzzed instead. ntp-proto may still serve as a test oracle, its server checking that our client interoperates.

## Unverified on Fargate

- Whether a Fargate task's kernel reports a synchronized clock. If it does, `SystemClock` takes `LinuxClock`, with a bound from the platform's own time sync.
- Whether a task can reach Amazon Time Sync at `169.254.169.123`. It smears leap seconds, so it can't join the public NTS servers in a quorum either way.

## After the merge

- Dispatch the `Clock platforms` workflow once: `LinuxClock` under chrony, and NTS from the public servers on Linux and Windows. It then runs weekly.
- Run `cargo bench -p sungma-clock` on Linux for `linux_now` at 32 threads. `LinuxClock` reads the kernel under its lock, and the cost under contention hasn't been measured.

## Checking Linux on Windows

`scripts/linux.sh` runs any cargo command for the Linux target in a podman container: clippy, the tests, `cargo mutants` on `linux.rs` and `time.rs`, and the benches. The container shares the kernel of podman's virtual machine, so `adjtimex` reads that machine's clock, which the container can't discipline. The weekly workflow stays the test of `LinuxClock` under chrony.
