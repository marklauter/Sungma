//! The Windows Time service's status, as `w32tm /query /status` prints it.
//! Parsed on every platform, so its tests run everywhere.

/// Half the root delay plus the root dispersion, in nanoseconds. `None`
/// when the service reports itself unsynchronized, or when the text lacks
/// a field, as it does in a language other than English, the only one
/// parsed.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn bound(status: &str) -> Option<u64> {
    let field = |name: &str| {
        status.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            (key.trim() == name).then(|| value.trim())
        })
    };
    let leap = field("Leap Indicator")?.split('(').next()?.trim();
    if leap.parse::<u8>().ok()? == 3 {
        return None;
    }
    let delay = seconds(field("Root Delay")?)?;
    let dispersion = seconds(field("Root Dispersion")?)?;
    Some((delay / 2).saturating_add(dispersion))
}

/// `0.0468750s` as nanoseconds.
fn seconds(value: &str) -> Option<u64> {
    let seconds: f64 = value.strip_suffix('s')?.parse().ok()?;
    // A float to integer cast saturates, so a huge value is u64::MAX.
    (seconds >= 0.0).then_some((seconds * 1e9) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYNCED: &str = "Leap Indicator: 0(no warning)
Stratum: 4 (secondary reference - syncd by (S)NTP)
Precision: -23 (119.209ns per tick)
Root Delay: 0.0468750s
Root Dispersion: 7.9431591s
ReferenceId: 0x14650039 (source IP:  20.101.57.9)
Last Successful Sync Time: 10/8/2026 9:12:44 AM
Source: time.windows.com,0x9
Poll Reading: 10 (1024s)
";

    #[test]
    fn the_bound_is_half_the_delay_plus_the_dispersion() {
        assert_eq!(bound(SYNCED), Some(23_437_500 + 7_943_159_100));
    }

    #[test]
    fn an_unsynchronized_service_has_no_bound() {
        let unsynced = SYNCED.replace("0(no warning)", "3(not synchronized)");
        assert_eq!(bound(&unsynced), None);
    }

    #[test]
    fn a_missing_or_unreadable_field_has_no_bound() {
        for (from, to) in [
            ("Leap Indicator", "Indicateur"),
            ("0(no warning)", "x(no warning)"),
            ("Root Delay", "Delai racine"),
            ("Root Dispersion", "Dispersion racine"),
            ("0.0468750s", "0.0468750"),
            ("7.9431591s", "-1s"),
        ] {
            assert_eq!(bound(&SYNCED.replace(from, to)), None, "{to}");
        }
    }

    #[test]
    fn a_huge_dispersion_saturates() {
        assert_eq!(
            bound(&SYNCED.replace("7.9431591s", "1e30s")),
            Some(u64::MAX)
        );
    }
}
