//! `format.ts`'s timestamps, in the local time zone. The reference defers to the browser's
//! locale; these are its output for the default `en-US` one.

use chrono::{Local, TimeZone};

fn local(taken_at: i64) -> Option<chrono::DateTime<Local>> {
    Local.timestamp_millis_opt(taken_at).single()
}

/// "Sep 22".
#[must_use]
pub fn day(taken_at: i64) -> String {
    local(taken_at).map_or_else(String::new, |time| time.format("%b %-d").to_string())
}

/// "04:02 PM".
#[must_use]
pub fn time(taken_at: i64) -> String {
    local(taken_at).map_or_else(String::new, |time| time.format("%I:%M %p").to_string())
}

/// "Sep 22, 04:02 PM".
#[must_use]
pub fn stamp(taken_at: i64) -> String {
    format!("{}, {}", day(taken_at), time(taken_at))
}

/// Whether two checkpoints fall on different calendar days, where the timeline draws a mark.
#[must_use]
pub fn is_new_day(previous: i64, next: i64) -> bool {
    local(previous).map(|time| time.date_naive()) != local(next).map(|time| time.date_naive())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stamp_joins_day_and_time() {
        let noon = Local
            .with_ymd_and_hms(2026, 9, 22, 16, 2, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        assert_eq!(stamp(noon), "Sep 22, 04:02 PM");
        assert!(!is_new_day(noon, noon + 60_000));
        assert!(is_new_day(noon, noon + 24 * 3_600_000));
    }
}
