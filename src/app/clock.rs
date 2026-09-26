//! Wall-clock stamps for reports (UTC, no time-zone database needed).

fn unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Current date and time as `YYYY-MM-DD HH:MM UTC`.
pub(super) fn utc_datetime() -> String {
    let s = unix_secs() as i64;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = s.div_euclid(86400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let secs = s.rem_euclid(86400);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        secs / 3600,
        (secs / 60) % 60
    )
}

/// Current time as `HH:MM:SS UTC`, to tell successive reports apart.
pub(super) fn utc_clock() -> String {
    let s = unix_secs();
    format!(
        "{:02}:{:02}:{:02} UTC",
        (s / 3600) % 24,
        (s / 60) % 60,
        s % 60
    )
}
