//! DATE_PART(field, source): one numeric field of a timestamp or interval.
//! `EXTRACT(field FROM source)` is rewritten onto this by the analyzer, so
//! both spellings share one implementation. Returns DOUBLE for every field.

use super::super::args::{arity, text, timestamp, KernelError};
use super::super::{Kernel, KernelCategory, KernelResult};
use crate::analyzer::Literal;
use chrono::{DateTime, Datelike, Duration, Timelike, Utc};

pub(super) const KERNEL: Kernel = Kernel {
    name: "DATE_PART",
    aliases: &["EXTRACT"],
    category: KernelCategory::Temporal,
    signature: "DATE_PART(field, timestamp | interval) -> DOUBLE",
    deterministic: true,
    strict: true,
    func: date_part,
};

fn date_part(args: &[Literal]) -> KernelResult {
    arity("DATE_PART", args, 2, 2)?;
    let field = text("DATE_PART", args, 0)?.trim().to_lowercase();
    let value = match &args[1] {
        Literal::Interval(d) => interval_part(&field, d)?,
        _ => timestamp_part(&field, timestamp("DATE_PART", args, 1)?)?,
    };
    Ok(Literal::Double(value))
}

pub(crate) fn timestamp_part(field: &str, ts: DateTime<Utc>) -> Result<f64, KernelError> {
    let seconds_with_fraction = ts.second() as f64 + ts.nanosecond() as f64 / 1e9;
    Ok(match field {
        "year" | "years" => ts.year() as f64,
        "month" | "months" => ts.month() as f64,
        "day" | "days" => ts.day() as f64,
        "hour" | "hours" => ts.hour() as f64,
        "minute" | "minutes" => ts.minute() as f64,
        "second" | "seconds" => seconds_with_fraction,
        "milliseconds" | "millisecond" => seconds_with_fraction * 1e3,
        "microseconds" | "microsecond" => seconds_with_fraction * 1e6,
        "epoch" => ts.timestamp() as f64 + ts.timestamp_subsec_nanos() as f64 / 1e9,
        "dow" | "dayofweek" => ts.weekday().num_days_from_sunday() as f64,
        "isodow" => ts.weekday().number_from_monday() as f64,
        "doy" | "dayofyear" => ts.ordinal() as f64,
        "week" | "weeks" | "isoweek" => ts.iso_week().week() as f64,
        "isoyear" => ts.iso_week().year() as f64,
        "quarter" => ((ts.month() - 1) / 3 + 1) as f64,
        "decade" => ts.year().div_euclid(10) as f64,
        "century" => century(ts.year()) as f64,
        "millennium" | "millenium" => millennium(ts.year()) as f64,
        "julian" => ts.timestamp() as f64 / 86_400.0 + 2_440_587.5,
        "timezone" | "timezone_hour" | "timezone_minute" => 0.0,
        _ => return Err(unknown_field(field)),
    })
}

fn century(year: i32) -> i32 {
    if year > 0 {
        (year - 1) / 100 + 1
    } else {
        -((-year) / 100 + 1)
    }
}

fn millennium(year: i32) -> i32 {
    if year > 0 {
        (year - 1) / 1000 + 1
    } else {
        -((-year) / 1000 + 1)
    }
}

/// An interval is an exact duration here, so `day` is whole days and the
/// time fields are the remainder within the day, like PostgreSQL's output.
fn interval_part(field: &str, d: &Duration) -> Result<f64, KernelError> {
    let micros = d.num_microseconds().unwrap_or(i64::MAX);
    let in_day = micros % 86_400_000_000;
    let in_hour = in_day % 3_600_000_000;
    let in_minute = in_hour % 60_000_000;
    Ok(match field {
        "epoch" => micros as f64 / 1e6,
        "day" | "days" => (micros / 86_400_000_000) as f64,
        "hour" | "hours" => (in_day / 3_600_000_000) as f64,
        "minute" | "minutes" => (in_hour / 60_000_000) as f64,
        "second" | "seconds" => in_minute as f64 / 1e6,
        "milliseconds" | "millisecond" => in_minute as f64 / 1e3,
        "microseconds" | "microsecond" => in_minute as f64,
        "year" | "years" | "month" | "months" | "quarter" | "decade" | "century" | "millennium" => {
            0.0
        }
        _ => return Err(unknown_field(field)),
    })
}

fn unknown_field(field: &str) -> KernelError {
    format!(
        "DATE_PART: unknown field '{}' (expected year, quarter, month, week, day, dow, isodow, \
         doy, hour, minute, second, milliseconds, microseconds, epoch, decade, century, \
         millennium, isoyear, julian)",
        field
    )
}
