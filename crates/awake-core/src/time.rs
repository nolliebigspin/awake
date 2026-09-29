use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// RFC 3339 UTC timestamp without pulling in a date crate.
pub fn utc_timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (y, m, d) = civil_from_days(days as i64);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Parse a duration like `30m`, `1h`, `1h30m` or `90s`. Units are required.
pub fn parse_duration(s: &str) -> Result<Duration, String> {
    const HINT: &str = "expected e.g. 30m, 1h, 1h30m";
    let s = s.trim();
    let mut total: u64 = 0;
    let mut digits = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        let unit = match c.to_ascii_lowercase() {
            'h' => 3600,
            'm' => 60,
            's' => 1,
            _ => return Err(format!("invalid duration {s:?}: {HINT}")),
        };
        let n: u64 = digits
            .parse()
            .map_err(|_| format!("invalid duration {s:?}: {HINT}"))?;
        total = n
            .checked_mul(unit)
            .and_then(|v| total.checked_add(v))
            .ok_or_else(|| format!("duration {s:?} is too long"))?;
        digits.clear();
    }
    if !digits.is_empty() || total == 0 {
        return Err(format!("invalid duration {s:?}: {HINT}"));
    }
    Ok(Duration::from_secs(total))
}

/// Human-readable remaining time: `2h 59m`, `45m`, or `30s` under a minute.
/// Rounds up, so a fresh 3h timer reads `3h` and the last second reads `1s`.
pub fn format_remaining(d: Duration) -> String {
    let secs = d.as_secs() + u64::from(d.subsec_nanos() > 0);
    if secs < 60 {
        return format!("{secs}s");
    }
    let mins = secs.div_ceil(60);
    match (mins / 60, mins % 60) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

/// Howard Hinnant's days-to-civil algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{civil_from_days, format_remaining, parse_duration};

    #[test]
    fn known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_725), (2026, 9, 29));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }

    #[test]
    fn parses_durations() {
        let ok = |s: &str| parse_duration(s).unwrap().as_secs();
        assert_eq!(ok("30m"), 1800);
        assert_eq!(ok("1h"), 3600);
        assert_eq!(ok("1h30m"), 5400);
        assert_eq!(ok("2H15M"), 8100);
        assert_eq!(ok("90s"), 90);
        assert_eq!(ok(" 24h "), 86_400);
        for bad in ["", "30", "0m", "h", "1x", "1h30", "-5m", "1.5h"] {
            assert!(parse_duration(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn formats_remaining() {
        let f = |s: u64| format_remaining(Duration::from_secs(s));
        assert_eq!(f(0), "0s");
        assert_eq!(f(59), "59s");
        assert_eq!(f(60), "1m");
        assert_eq!(f(61), "2m");
        assert_eq!(f(45 * 60), "45m");
        assert_eq!(f(3600), "1h");
        assert_eq!(f(3 * 3600 - 30), "3h");
        assert_eq!(f(3 * 3600 - 61), "2h 59m");
        let ms = |ms: u64| format_remaining(Duration::from_millis(ms));
        assert_eq!(ms(300), "1s");
        assert_eq!(ms(58_300), "59s");
        assert_eq!(ms(59_300), "1m");
    }
}
