//! Timezone-aware cron evaluator (5-field: min hour dom month dow).

use chrono::{DateTime, Datelike, Timelike, Utc};

/// Parse a timezone string into a `chrono_tz::Tz`, falling back to UTC.
pub fn parse_tz(tz: &str) -> chrono_tz::Tz {
    tz.parse::<chrono_tz::Tz>().unwrap_or(chrono_tz::UTC)
}

/// Parse a cron field and check if a value matches.
fn cron_field_matches(field: &str, value: u32) -> bool {
    if field == "*" {
        return true;
    }
    // Handle */N (every N)
    if let Some(step) = field.strip_prefix("*/") {
        if let Ok(n) = step.parse::<u32>() {
            return n > 0 && value.is_multiple_of(n);
        }
    }
    // Handle comma-separated values
    for part in field.split(',') {
        // Handle range N-M
        if let Some((start_s, end_s)) = part.split_once('-') {
            if let (Ok(start), Ok(end)) = (start_s.parse::<u32>(), end_s.parse::<u32>()) {
                if value >= start && value <= end {
                    return true;
                }
            }
        } else if let Ok(n) = part.parse::<u32>() {
            if value == n {
                return true;
            }
        }
    }
    false
}

/// Check if a **local** naive datetime matches a 5-field cron expression.
fn cron_matches_naive(cron: &str, dt: &chrono::NaiveDateTime) -> bool {
    let fields: Vec<&str> = cron.split_whitespace().collect();
    if fields.len() != 5 {
        return false;
    }
    cron_field_matches(fields[0], dt.minute())
        && cron_field_matches(fields[1], dt.hour())
        && cron_field_matches(fields[2], dt.day())
        && cron_field_matches(fields[3], dt.month())
        && cron_field_matches(fields[4], dt.weekday().num_days_from_sunday())
}

/// Check if a UTC datetime matches a 5-field cron expression (UTC shorthand).
pub fn cron_matches(cron: &str, dt: &DateTime<Utc>) -> bool {
    cron_matches_naive(cron, &dt.naive_utc())
}

/// Compute next occurrence after `after` for a cron expression, evaluated in
/// the given timezone. Returns a UTC `DateTime`.
///
/// **DST handling:**
/// - Spring-forward gaps: local times that don't exist are skipped.
/// - Fall-back overlaps: the earliest (pre-transition) mapping is chosen.
pub fn cron_next_tz(cron: &str, after: &DateTime<Utc>, tz: chrono_tz::Tz) -> Option<DateTime<Utc>> {
    use chrono::TimeZone;

    // Convert `after` to local time and advance to the next whole minute.
    let local_after = after.with_timezone(&tz).naive_local();
    let next_min_secs = 60 - (local_after.second() as i64);
    let mut candidate = local_after + chrono::Duration::seconds(next_min_secs);
    candidate = candidate.with_second(0).unwrap_or(candidate);

    let max_checks = 366 * 24 * 60; // one year of minutes
    for _ in 0..max_checks {
        if cron_matches_naive(cron, &candidate) {
            // Convert back to UTC. If this local time is in a DST gap
            // (doesn't exist), skip it.
            match tz.from_local_datetime(&candidate) {
                chrono::LocalResult::Single(dt) => return Some(dt.with_timezone(&Utc)),
                chrono::LocalResult::Ambiguous(earliest, _) => {
                    return Some(earliest.with_timezone(&Utc));
                }
                chrono::LocalResult::None => {
                    // DST gap — this local minute doesn't exist. Skip.
                }
            }
        }
        candidate += chrono::Duration::minutes(1);
    }
    None
}

/// Convenience: compute next occurrence using UTC (for backward compat).
pub fn cron_next(cron: &str, after: &DateTime<Utc>) -> Option<DateTime<Utc>> {
    cron_next_tz(cron, after, chrono_tz::UTC)
}

/// Compute up to N next occurrences, timezone-aware.
pub fn cron_next_n_tz(
    cron: &str,
    after: &DateTime<Utc>,
    n: usize,
    tz: chrono_tz::Tz,
) -> Vec<DateTime<Utc>> {
    let mut results = Vec::with_capacity(n);
    let mut cursor = *after;
    for _ in 0..n {
        match cron_next_tz(cron, &cursor, tz) {
            Some(next) => {
                results.push(next);
                cursor = next;
            }
            None => break,
        }
    }
    results
}

/// Convenience: compute up to N next occurrences using UTC.
pub fn cron_next_n(cron: &str, after: &DateTime<Utc>, n: usize) -> Vec<DateTime<Utc>> {
    cron_next_n_tz(cron, after, n, chrono_tz::UTC)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn cron_every_5_minutes() {
        let dt = Utc.with_ymd_and_hms(2024, 6, 15, 10, 0, 0).unwrap();
        assert!(cron_matches("*/5 * * * *", &dt));
        let dt2 = Utc.with_ymd_and_hms(2024, 6, 15, 10, 3, 0).unwrap();
        assert!(!cron_matches("*/5 * * * *", &dt2));
    }

    #[test]
    fn cron_specific_time() {
        let dt = Utc.with_ymd_and_hms(2024, 6, 15, 9, 30, 0).unwrap();
        assert!(cron_matches("30 9 * * *", &dt));
        assert!(!cron_matches("30 10 * * *", &dt));
    }

    #[test]
    fn cron_range() {
        let dt = Utc.with_ymd_and_hms(2024, 6, 15, 10, 0, 0).unwrap();
        assert!(cron_matches("0 9-17 * * *", &dt));
        let dt2 = Utc.with_ymd_and_hms(2024, 6, 15, 20, 0, 0).unwrap();
        assert!(!cron_matches("0 9-17 * * *", &dt2));
    }

    #[test]
    fn cron_next_finds_occurrence() {
        let after = Utc.with_ymd_and_hms(2024, 6, 15, 10, 0, 0).unwrap();
        let next = cron_next("30 * * * *", &after);
        assert!(next.is_some());
        let next = next.unwrap();
        assert_eq!(next.minute(), 30);
    }

    #[test]
    fn cron_next_n_returns_multiple() {
        let after = Utc.with_ymd_and_hms(2024, 6, 15, 10, 0, 0).unwrap();
        let results = cron_next_n("0 * * * *", &after, 5);
        assert_eq!(results.len(), 5);
    }

    #[test]
    fn cron_comma_separated() {
        let dt = Utc.with_ymd_and_hms(2024, 6, 15, 10, 15, 0).unwrap();
        assert!(cron_matches("0,15,30,45 * * * *", &dt));
        let dt2 = Utc.with_ymd_and_hms(2024, 6, 15, 10, 20, 0).unwrap();
        assert!(!cron_matches("0,15,30,45 * * * *", &dt2));
    }

    // ── Timezone-aware cron tests ─────────────────────────────────────

    #[test]
    fn cron_next_tz_basic() {
        let after = Utc.with_ymd_and_hms(2024, 6, 15, 12, 0, 0).unwrap();
        let tz = parse_tz("US/Eastern");
        let next = cron_next_tz("0 9 * * *", &after, tz).unwrap();
        assert_eq!(next.hour(), 13); // 9 ET = 13 UTC (EDT is UTC-4)
        assert_eq!(next.minute(), 0);
    }

    #[test]
    fn cron_next_tz_spring_forward() {
        let after = Utc.with_ymd_and_hms(2024, 3, 10, 6, 0, 0).unwrap();
        let tz = parse_tz("US/Eastern");
        let next = cron_next_tz("30 2 * * *", &after, tz).unwrap();
        assert_eq!(next.day(), 11);
        assert_eq!(next.hour(), 6);
        assert_eq!(next.minute(), 30);
    }

    #[test]
    fn cron_next_tz_fall_back() {
        let after = Utc.with_ymd_and_hms(2024, 11, 3, 4, 0, 0).unwrap();
        let tz = parse_tz("US/Eastern");
        let next = cron_next_tz("30 1 * * *", &after, tz).unwrap();
        assert_eq!(next.hour(), 5);
        assert_eq!(next.minute(), 30);
    }

    #[test]
    fn cron_next_tz_invalid_falls_back_to_utc() {
        let after = Utc.with_ymd_and_hms(2024, 6, 15, 10, 0, 0).unwrap();
        let tz = parse_tz("Invalid/Timezone");
        let next = cron_next_tz("30 * * * *", &after, tz).unwrap();
        assert_eq!(next.minute(), 30);
        assert_eq!(next.hour(), 10);
    }

    #[test]
    fn cron_next_n_tz_produces_correct_utc_times() {
        let after = Utc.with_ymd_and_hms(2024, 6, 15, 0, 0, 0).unwrap();
        let tz = parse_tz("Asia/Tokyo");
        let results = cron_next_n_tz("0 9 * * *", &after, 3, tz);
        assert_eq!(results.len(), 3);
        for r in &results {
            assert_eq!(r.hour(), 0); // 9 JST = 0 UTC
            assert_eq!(r.minute(), 0);
        }
    }

    #[test]
    fn parse_tz_valid() {
        assert_eq!(parse_tz("America/New_York"), chrono_tz::America::New_York);
        assert_eq!(parse_tz("UTC"), chrono_tz::UTC);
        assert_eq!(parse_tz("Europe/London"), chrono_tz::Europe::London);
    }

    #[test]
    fn parse_tz_invalid_returns_utc() {
        assert_eq!(parse_tz("Not/Real"), chrono_tz::UTC);
        assert_eq!(parse_tz(""), chrono_tz::UTC);
    }
}
