//! Reads the clock every 5 seconds and logs each reading, and the
//! clock's status, to stdout.
//!
//! `cargo run -p sungma-clock --example watch [-- host ...]`
//!
//! The clock is chosen as production chooses it: the Linux kernel's own
//! when chrony has synced it, else NTS against the hosts given, or the
//! public NTS servers when none are. It refreshes every 64 seconds in the
//! background.

use std::{env, error::Error, sync::Arc, time::Duration};

use sungma::clock::{Clock, Reading};
use sungma_clock::{ClockStatus, Servers, SystemClock, refresher};

/// Public NTS servers from independent operators, none of which smears
/// leap seconds.
const SERVERS: [&str; 4] = [
    "time.cloudflare.com",
    "nts.netnod.se",
    "ptbtime1.ptb.de",
    "nts.time.nl",
];

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let hosts: Vec<String> = env::args().skip(1).collect();
    let servers = if hosts.is_empty() {
        Servers::nts(SERVERS)
    } else {
        Servers::nts(hosts)
    };
    let clock = Arc::new(tokio::task::spawn_blocking(move || SystemClock::detect(servers)).await??);
    match clock.as_ref() {
        #[cfg(target_os = "linux")]
        SystemClock::Linux(_) => println!("clock: the Linux kernel's, synced by chrony"),
        SystemClock::Ntp(_) => println!("clock: NTP, authenticated with NTS"),
    }
    let _refreshing = refresher(clock.clone(), |result| {
        if let Err(failed) = result {
            println!("refresh failed: {failed}");
        }
    });
    let mut ticks = tokio::time::interval(Duration::from_secs(5));
    loop {
        ticks.tick().await;
        match clock.now() {
            Ok(reading) => println!("{}", line(reading)),
            Err(fault) => println!("fault: {fault}"),
        }
        println!("  {}", status(&clock.status()));
    }
}

/// `sample 12.3 s ago · faults 0 · failed syncs 0`, then whatever the
/// last sync found that wasn't routine.
fn status(status: &ClockStatus) -> String {
    let faults = &status.faults;
    let mut parts = Vec::new();
    if let Some(since) = status.since_sample {
        parts.push(format!("sample {:.1} s ago", since.as_secs_f64()));
    }
    parts.push(format!(
        "faults {} (jumped {}, drifted {}, behind {}, unsynchronized {})",
        faults.jumped + faults.drifted + faults.behind + faults.unsynchronized,
        faults.jumped,
        faults.drifted,
        faults.behind,
        faults.unsynchronized
    ));
    parts.push(format!("failed syncs {}", status.failed_syncs));
    if let Some(drift) = status.drift {
        let ppm = |ppb: i64| ppb as f64 / 1_000.0;
        let allowance = ppm(drift.allowance as i64);
        parts.push(match drift.measured {
            Some((low, high)) => format!(
                "drift {:.1}..{:.1} ppm (allowance {allowance:.1} ppm)",
                ppm(low),
                ppm(high)
            ),
            None => format!("drift unmeasured (allowance {allowance:.0} ppm)"),
        });
    }
    if let Some(sync) = &status.last_sync {
        let mut listed = |name: &str, servers: &[String]| {
            if !servers.is_empty() {
                parts.push(format!("{name} {}", servers.join(", ")));
            }
        };
        listed("falsetickers", &sync.falsetickers);
        listed("unanswered", &sync.unanswered);
        listed("denied", &sync.denied);
        listed("overridden", &sync.overridden);
        let kissed: Vec<String> = sync
            .kissed
            .iter()
            .map(|(server, code)| format!("{server} {}", String::from_utf8_lossy(code)))
            .collect();
        listed("kissed", &kissed);
        if let Some(leap) = sync.leap {
            parts.push(format!("leap second {leap:?}"));
        }
    }
    parts.join(" · ")
}

/// `2026-10-08T14:03:27.512345678Z ±11.204 ms  settled … revision …`
fn line(reading: Reading) -> String {
    let (settled, revision) = (reading.settled.0, reading.revision.0);
    let middle = settled + (revision - settled) / 2;
    let bound = (revision - settled) as f64 / 2e6;
    format!(
        "{} ±{bound:.3} ms  settled {settled} revision {revision}",
        utc(middle)
    )
}

/// Nanoseconds since the Unix epoch as an ISO 8601 UTC time.
fn utc(nanos: u64) -> String {
    let seconds = nanos / 1_000_000_000;
    let (days, of_day) = (seconds / 86_400, seconds % 86_400);
    let (year, month, day) = civil(i64::try_from(days).unwrap_or(i64::MAX));
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:09}Z",
        of_day / 3_600,
        of_day / 60 % 60,
        of_day % 60,
        nanos % 1_000_000_000
    )
}

/// The proleptic Gregorian date `days` after 1970-01-01, by Howard
/// Hinnant's `civil_from_days`.
fn civil(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let of_era = shifted.rem_euclid(146_097);
    let year_of_era = (of_era - of_era / 1_460 + of_era / 36_524 - of_era / 146_096) / 365;
    let of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * of_year + 2) / 153;
    let day = of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month as u32, day as u32)
}
