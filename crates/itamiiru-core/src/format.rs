//! Human-readable sizes and dates.

use std::time::{SystemTime, UNIX_EPOCH};

/// "0 bytes", "512 bytes", "1.5 KB", "12 MB". Units are 1024-based, as on Windows.
pub fn format_size(bytes: u64) -> String {
    if bytes == 1 {
        return "1 byte".to_string();
    }
    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    let units = ["KB", "MB", "GB", "TB", "PB", "EB"];
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value < 10.0 {
        return format!("{:.1} {}", value, units[unit]);
    }
    return format!("{:.0} {}", value, units[unit]);
}

/// "2026-10-11 15:42" in the local time zone.
pub fn format_local_time(time: SystemTime) -> String {
    let secs = unix_seconds(time);
    let offset = crate::platform::utc_offset_seconds(secs);
    return format_unix_seconds(secs.saturating_add(offset));
}

/// Seconds since 1970-01-01 UTC; negative for earlier times.
fn unix_seconds(time: SystemTime) -> i64 {
    return match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_secs()).unwrap_or(i64::MAX),
        Err(before) => -i64::try_from(before.duration().as_secs()).unwrap_or(i64::MAX),
    };
}

/// Formats seconds since the epoch as "YYYY-MM-DD HH:MM", with no time zone math.
fn format_unix_seconds(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = secs_of_day / 3600;
    let minute = secs_of_day % 3600 / 60;
    return format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}");
}

/// Days since 1970-01-01 to (year, month, day) in the Gregorian calendar.
/// Howard Hinnant's algorithm: https://howardhinnant.github.io/date_algorithms.html
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097; // day of era, 0..=146096
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // year of era
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year, March-based
    let mp = (5 * doy + 2) / 153; // month, March = 0
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    return (year, month, day);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(format_size(0), "0 bytes");
        assert_eq!(format_size(1), "1 byte");
        assert_eq!(format_size(1023), "1023 bytes");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(20 * 1024), "20 KB");
        assert_eq!(format_size(3 * 1024 * 1024), "3.0 MB");
        assert_eq!(format_size(u64::MAX), "16 EB");
    }

    #[test]
    fn utc_dates() {
        assert_eq!(format_unix_seconds(0), "1970-01-01 00:00");
        assert_eq!(format_unix_seconds(951_782_400), "2000-02-29 00:00");
        assert_eq!(format_unix_seconds(1_791_740_520), "2026-10-11 17:42");
        assert_eq!(format_unix_seconds(-1), "1969-12-31 23:59");
        assert_eq!(format_unix_seconds(253_402_300_799), "9999-12-31 23:59");
    }

    #[test]
    fn extreme_times_dont_panic() {
        format_unix_seconds(i64::MAX);
        format_unix_seconds(i64::MIN);
        format_local_time(UNIX_EPOCH);
    }
}
