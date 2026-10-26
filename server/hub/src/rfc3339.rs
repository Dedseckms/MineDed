//! Minimal RFC 3339 (UTC) formatting without a date/time dependency.

use std::time::{SystemTime, UNIX_EPOCH};

/// Formats `t` as `YYYY-MM-DDTHH:MM:SSZ` (UTC, second precision).
pub fn format(t: SystemTime) -> String {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Next 1st-of-month 00:00 in a fixed-offset zone (`utc_offset_secs` east of UTC), as UTC.
/// An instant exactly at that midnight yields the following month.
pub fn next_month_start(now: SystemTime, utc_offset_secs: i64) -> SystemTime {
    let secs = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let local_days = (secs + utc_offset_secs).div_euclid(86_400);
    let (y, m, _) = civil_from_days(local_days);
    let (y, m) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    let next = days_from_civil(y, m, 1) * 86_400 - utc_offset_secs;
    UNIX_EPOCH + std::time::Duration::from_secs(next.max(0) as u64)
}

/// Inverse of `civil_from_days` (Howard Hinnant).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 -> (year, month, day).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn formats_known_instants() {
        assert_eq!(format(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        let t = UNIX_EPOCH + Duration::from_secs(951_782_400); // leap day
        assert_eq!(format(t), "2000-02-29T00:00:00Z");
        let t = UNIX_EPOCH + Duration::from_secs(1_792_627_200 + 3_723);
        assert_eq!(format(t), "2026-10-22T01:02:03Z");
    }

    #[test]
    fn next_month_start_in_msk() {
        let msk = 3 * 3600;
        // 2026-10-22T01:02:03Z -> 2026-11-01T00:00+03:00.
        let t = UNIX_EPOCH + Duration::from_secs(1_792_627_200 + 3_723);
        assert_eq!(format(next_month_start(t, msk)), "2026-10-31T21:00:00Z");
        // 2026-12-31T22:00Z is already Jan 1st in MSK -> Feb 1st.
        let t = UNIX_EPOCH + Duration::from_secs(1_798_754_400);
        assert_eq!(format(t), "2026-12-31T22:00:00Z");
        assert_eq!(format(next_month_start(t, msk)), "2027-01-31T21:00:00Z");
        assert_eq!(
            format(next_month_start(UNIX_EPOCH, 0)),
            "1970-02-01T00:00:00Z"
        );
    }
}
