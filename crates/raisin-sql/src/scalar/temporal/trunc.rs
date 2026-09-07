//! DATE_TRUNC(unit, timestamp): the start of the unit containing the instant,
//! in UTC. Weeks start on Monday, as in PostgreSQL.

use super::super::args::{arity, text, timestamp, KernelError};
use super::super::{Kernel, KernelCategory, KernelResult};
use crate::analyzer::Literal;
use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc};

pub(super) const KERNEL: Kernel = Kernel {
    name: "DATE_TRUNC",
    aliases: &[],
    category: KernelCategory::Temporal,
    signature: "DATE_TRUNC(unit, timestamp) -> TIMESTAMPTZ",
    deterministic: true,
    strict: true,
    func: date_trunc,
};

fn date_trunc(args: &[Literal]) -> KernelResult {
    arity("DATE_TRUNC", args, 2, 2)?;
    let unit = text("DATE_TRUNC", args, 0)?;
    let ts = timestamp("DATE_TRUNC", args, 1)?;
    Ok(Literal::Timestamp(truncate(unit, ts)?))
}

/// Truncate `ts` to `unit`. Public within the crate so `CAST(x AS DATE)` and
/// AGE share it.
pub(crate) fn truncate(unit: &str, ts: DateTime<Utc>) -> Result<DateTime<Utc>, KernelError> {
    let unit = unit.trim().to_lowercase();
    let year_start = |y: i32| Utc.with_ymd_and_hms(y, 1, 1, 0, 0, 0).single();
    let clear_time = |ts: DateTime<Utc>| ts.with_nanosecond(0);
    let out = match unit.trim_end_matches('s') {
        "microsecond" => Some(ts),
        "millisecond" => ts.with_nanosecond(ts.nanosecond() / 1_000_000 * 1_000_000),
        "second" => clear_time(ts),
        "minute" => clear_time(ts).and_then(|t| t.with_second(0)),
        "hour" => clear_time(ts)
            .and_then(|t| t.with_second(0))
            .and_then(|t| t.with_minute(0)),
        "day" => Some(super::midnight(ts)),
        "week" => {
            let days_since_monday = ts.weekday().num_days_from_monday() as i64;
            Some(super::midnight(ts) - Duration::days(days_since_monday))
        }
        "month" => Utc
            .with_ymd_and_hms(ts.year(), ts.month(), 1, 0, 0, 0)
            .single(),
        "quarter" => {
            let month = (ts.month() - 1) / 3 * 3 + 1;
            Utc.with_ymd_and_hms(ts.year(), month, 1, 0, 0, 0).single()
        }
        "year" => year_start(ts.year()),
        "decade" => year_start(ts.year().div_euclid(10) * 10),
        "century" => year_start((ts.year() - 1).div_euclid(100) * 100 + 1),
        "millennium" => year_start((ts.year() - 1).div_euclid(1000) * 1000 + 1),
        _ => {
            return Err(format!(
                "DATE_TRUNC: unknown unit '{}' (expected microseconds, milliseconds, second, \
                 minute, hour, day, week, month, quarter, year, decade, century, millennium)",
                unit
            ))
        }
    };
    out.ok_or_else(|| format!("DATE_TRUNC: cannot truncate {} to {}", ts, unit))
}
