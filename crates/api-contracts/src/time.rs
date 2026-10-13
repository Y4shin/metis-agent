//! The time format the wire contract asks for.
//!
//! `EchoResponse.received_at` is an RFC 3339 string, and both the test-support
//! skeleton and the daemon's own service answer that field. The format lives
//! here so the two crates that serve the contract share one implementation
//! rather than each carrying its own calendar arithmetic.

use std::time::{SystemTime, UNIX_EPOCH};

/// The current time as an RFC 3339 UTC stamp, `YYYY-MM-DDThh:mm:ssZ`.
///
/// The value is always UTC and carries no fractional seconds, which is all the
/// contract asks for: no leap-second or timezone handling is involved.
#[must_use]
pub fn rfc3339_now() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since_epoch| since_epoch.as_secs())
        .unwrap_or(0);
    rfc3339(seconds)
}

/// Unix seconds as an RFC 3339 UTC stamp.
#[must_use]
pub fn rfc3339(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let second_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = second_of_day / 3600;
    let minute = (second_of_day % 3600) / 60;
    let second = second_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Days since 1970-01-01 to a proleptic Gregorian `(year, month, day)`.
///
/// This is Howard Hinnant's `civil_from_days`, the standard days-to-calendar
/// algorithm.
fn civil_from_days(days: i64) -> (i64, u64, u64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = (z - era * 146_097) as u64; // [0, 146096]
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153; // [0, 11]
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1; // [1, 31]
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    }; // [1, 12]
    (year + i64::from(month <= 2), month, day)
}

#[cfg(test)]
mod tests {
    use super::rfc3339;

    /// The stamp of a known instant, worked out from the calendar rather than
    /// from the code's own arithmetic.
    #[test]
    fn a_known_instant_formats_as_an_rfc3339_stamp() {
        // 2026-01-02T03:04:05Z is 1767323045 seconds after the Unix epoch.
        assert_eq!(rfc3339(1_767_323_045), "2026-01-02T03:04:05Z");
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        // 2000-02-29T12:00:00Z, a leap day in a leap century.
        assert_eq!(rfc3339(951_825_600), "2000-02-29T12:00:00Z");
    }
}
