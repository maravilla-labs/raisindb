//! IANA time-zone conversion for the function sandbox.
//!
//! QuickJS is built without `Intl`, so `raisin.date` is the only way a
//! function can ask what a wall-clock time in a named zone means as an
//! instant. Everything a schedule needs — "every Tuesday 09:00
//! Europe/Zurich", surviving both DST transitions — comes through here, so
//! the two awkward cases are decided once, in one place, with tests.

use chrono::{Datelike, Duration, LocalResult, NaiveDate, Offset, TimeZone, Timelike};
use raisin_error::{Error, Result};
use serde_json::{json, Value};

fn zone(name: &str) -> Result<chrono_tz::Tz> {
    name.parse()
        .map_err(|_| Error::internal(format!("Unknown time zone: {name}")))
}

/// Wall-clock fields of an instant in a named IANA zone.
pub(crate) fn to_zone(timestamp: i64, time_zone: &str) -> Result<Value> {
    let tz = zone(time_zone)?;
    let utc = chrono::DateTime::from_timestamp(timestamp, 0)
        .ok_or_else(|| Error::internal("Timestamp out of range"))?;
    let local = utc.with_timezone(&tz);
    Ok(json!({
        "year": local.year(),
        "month": local.month(),
        "day": local.day(),
        "hour": local.hour(),
        "minute": local.minute(),
        "second": local.second(),
        // 0 = Sunday, matching JavaScript's getDay() so a cron day-of-week
        // field means the same thing on both sides of this boundary.
        "weekday": local.weekday().num_days_from_sunday(),
        "offset_minutes": local.offset().fix().local_minus_utc() / 60,
        "zone": time_zone,
    }))
}

/// The UTC instant for a wall-clock time in a named IANA zone.
///
/// A LOCAL TIME IS NOT ALWAYS ONE INSTANT, and refusing the awkward ones
/// would mean a daily rule silently skips the day the clocks change.
///   - Ambiguous (autumn, the hour that runs twice): take the EARLIER, so a
///     09:00 rule fires once and at the first 09:00, not the second.
///   - Skipped (spring, the hour that never happens): take the instant the
///     clock jumps TO, by asking for the same wall time an hour later.
pub(crate) fn from_zone(
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    time_zone: &str,
) -> Result<i64> {
    let tz = zone(time_zone)?;
    let naive = NaiveDate::from_ymd_opt(year as i32, month as u32, day as u32)
        .and_then(|d| d.and_hms_opt(hour as u32, minute as u32, second as u32))
        .ok_or_else(|| {
            Error::internal(format!(
                "Not a date: {year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}"
            ))
        })?;
    let resolved = match tz.from_local_datetime(&naive) {
        LocalResult::Single(dt) => dt,
        LocalResult::Ambiguous(earlier, _later) => earlier,
        LocalResult::None => {
            let shifted = naive + Duration::hours(1);
            tz.from_local_datetime(&shifted).earliest().ok_or_else(|| {
                Error::internal("This wall-clock time does not exist in this zone.")
            })?
        }
    };
    Ok(resolved.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(v: &Value, k: &str) -> i64 {
        v.get(k).and_then(Value::as_i64).unwrap()
    }

    #[test]
    fn to_zone_reports_the_offset_the_zone_actually_had() {
        // 2026-01-15 12:00 UTC — Zurich on standard time, UTC+1.
        let winter = to_zone(1_768_478_400, "Europe/Zurich").unwrap();
        assert_eq!(field(&winter, "hour"), 13);
        assert_eq!(field(&winter, "offset_minutes"), 60);

        // 2026-07-15 12:00 UTC — Zurich on summer time, UTC+2.
        let summer = to_zone(1_784_116_800, "Europe/Zurich").unwrap();
        assert_eq!(field(&summer, "hour"), 14);
        assert_eq!(field(&summer, "offset_minutes"), 120);
    }

    #[test]
    fn weekday_is_zero_for_sunday_like_javascript() {
        // 2026-01-18 is a Sunday.
        let v = to_zone(from_zone(2026, 1, 18, 12, 0, 0, "UTC").unwrap(), "UTC").unwrap();
        assert_eq!(field(&v, "weekday"), 0);
        let v = to_zone(from_zone(2026, 1, 20, 12, 0, 0, "UTC").unwrap(), "UTC").unwrap();
        assert_eq!(field(&v, "weekday"), 2); // Tuesday
    }

    /// The whole point of the binding: the same wall-clock rule is a
    /// DIFFERENT UTC instant on either side of the transition. Arithmetic on
    /// a fixed offset gets one of these two wrong.
    #[test]
    fn nine_am_zurich_is_a_different_instant_in_summer_and_winter() {
        let winter = from_zone(2026, 1, 20, 9, 0, 0, "Europe/Zurich").unwrap();
        let summer = from_zone(2026, 7, 21, 9, 0, 0, "Europe/Zurich").unwrap();
        assert_eq!(field(&to_zone(winter, "UTC").unwrap(), "hour"), 8);
        assert_eq!(field(&to_zone(summer, "UTC").unwrap(), "hour"), 7);
    }

    #[test]
    fn a_skipped_wall_time_moves_forward_instead_of_failing() {
        // 2026-03-29, Zurich jumps 02:00 -> 03:00. 02:30 never happens.
        let ts = from_zone(2026, 3, 29, 2, 30, 0, "Europe/Zurich").unwrap();
        let back = to_zone(ts, "Europe/Zurich").unwrap();
        assert_eq!(field(&back, "hour"), 3);
        assert_eq!(field(&back, "minute"), 30);
        assert_eq!(field(&back, "offset_minutes"), 120);
    }

    #[test]
    fn an_ambiguous_wall_time_resolves_to_the_earlier_instant() {
        // 2026-10-25, Zurich repeats 02:00-03:00. 02:30 happens twice.
        let ts = from_zone(2026, 10, 25, 2, 30, 0, "Europe/Zurich").unwrap();
        let back = to_zone(ts, "Europe/Zurich").unwrap();
        assert_eq!(field(&back, "hour"), 2);
        // The earlier one is still on summer time, UTC+2.
        assert_eq!(field(&back, "offset_minutes"), 120);
        // An hour later is the SECOND 02:30 — same wall clock, winter offset.
        let second = to_zone(ts + 3600, "Europe/Zurich").unwrap();
        assert_eq!(field(&second, "hour"), 2);
        assert_eq!(field(&second, "minute"), 30);
        assert_eq!(field(&second, "offset_minutes"), 60);
    }

    #[test]
    fn a_southern_hemisphere_zone_transitions_the_other_way() {
        // Auckland is UTC+13 in January (summer) and UTC+12 in July.
        let jan = to_zone(1_768_478_400, "Pacific/Auckland").unwrap();
        assert_eq!(field(&jan, "offset_minutes"), 780);
        let jul = to_zone(1_784_116_800, "Pacific/Auckland").unwrap();
        assert_eq!(field(&jul, "offset_minutes"), 720);
    }

    #[test]
    fn a_half_hour_zone_keeps_its_thirty_minutes() {
        let v = to_zone(1_768_478_400, "Asia/Kolkata").unwrap();
        assert_eq!(field(&v, "offset_minutes"), 330);
        assert_eq!(field(&v, "hour"), 17);
        assert_eq!(field(&v, "minute"), 30);
    }

    #[test]
    fn an_unknown_zone_is_an_error_not_a_silent_utc() {
        assert!(to_zone(0, "Mars/Olympus").is_err());
        assert!(from_zone(2026, 1, 1, 0, 0, 0, "Mars/Olympus").is_err());
    }

    #[test]
    fn an_impossible_date_is_an_error() {
        assert!(from_zone(2026, 2, 30, 0, 0, 0, "UTC").is_err());
        assert!(from_zone(2026, 1, 1, 25, 0, 0, "UTC").is_err());
    }
}
