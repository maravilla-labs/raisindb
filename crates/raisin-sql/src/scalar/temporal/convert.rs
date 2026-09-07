//! Constructors: TO_TIMESTAMP, TO_DATE, MAKE_DATE, MAKE_TIMESTAMP.
//!
//! `TO_TIMESTAMP(text, format)` parses with the same pattern grammar
//! `TO_CHAR` renders with (see `to_char::tokenize`), so a round trip through
//! the two is lossless for every supported field.

use super::super::args::{arity, int, num, text, KernelError};
use super::super::{Kernel, KernelCategory, KernelResult};
use super::to_char::{tokenize, Token};
use crate::analyzer::Literal;
use chrono::{DateTime, NaiveDate, TimeZone, Utc};

macro_rules! kernel {
    ($name:expr, $sig:expr, $f:expr) => {
        Kernel {
            name: $name,
            aliases: &[],
            category: KernelCategory::Temporal,
            signature: $sig,
            deterministic: true,
            strict: true,
            func: $f,
        }
    };
}

pub(super) const TO_TIMESTAMP: Kernel = kernel!(
    "TO_TIMESTAMP",
    "TO_TIMESTAMP(epoch_seconds | text, format) -> TIMESTAMPTZ",
    to_timestamp
);
pub(super) const TO_DATE: Kernel = kernel!(
    "TO_DATE",
    "TO_DATE(text, format) -> TIMESTAMPTZ (midnight UTC)",
    to_date
);
pub(super) const MAKE_DATE: Kernel = kernel!(
    "MAKE_DATE",
    "MAKE_DATE(year, month, day) -> TIMESTAMPTZ (midnight UTC)",
    make_date
);
pub(super) const MAKE_TIMESTAMP: Kernel = kernel!(
    "MAKE_TIMESTAMP",
    "MAKE_TIMESTAMP(year, month, day, hour, minute, seconds) -> TIMESTAMPTZ",
    make_timestamp
);

fn to_timestamp(args: &[Literal]) -> KernelResult {
    arity("TO_TIMESTAMP", args, 1, 2)?;
    if args.len() == 1 {
        let secs = num("TO_TIMESTAMP", args, 0)?;
        let whole = secs.floor();
        let nanos = ((secs - whole) * 1e9).round() as u32;
        return Utc
            .timestamp_opt(whole as i64, nanos)
            .single()
            .map(Literal::Timestamp)
            .ok_or_else(|| format!("TO_TIMESTAMP: {} is out of range", secs));
    }
    let parsed = parse_with_pattern(
        "TO_TIMESTAMP",
        text("TO_TIMESTAMP", args, 0)?,
        text("TO_TIMESTAMP", args, 1)?,
    )?;
    Ok(Literal::Timestamp(parsed))
}

fn to_date(args: &[Literal]) -> KernelResult {
    arity("TO_DATE", args, 2, 2)?;
    let parsed = parse_with_pattern(
        "TO_DATE",
        text("TO_DATE", args, 0)?,
        text("TO_DATE", args, 1)?,
    )?;
    Ok(Literal::Timestamp(super::midnight(parsed)))
}

fn ymd(name: &str, y: i64, m: i64, d: i64) -> Result<NaiveDate, KernelError> {
    let (Ok(y), Ok(m), Ok(d)) = (i32::try_from(y), u32::try_from(m), u32::try_from(d)) else {
        return Err(format!("{}: date field out of range", name));
    };
    NaiveDate::from_ymd_opt(y, m, d)
        .ok_or_else(|| format!("{}: {}-{:02}-{:02} is not a valid date", name, y, m, d))
}

fn make_date(args: &[Literal]) -> KernelResult {
    arity("MAKE_DATE", args, 3, 3)?;
    let date = ymd(
        "MAKE_DATE",
        int("MAKE_DATE", args, 0)?,
        int("MAKE_DATE", args, 1)?,
        int("MAKE_DATE", args, 2)?,
    )?;
    Ok(Literal::Timestamp(
        date.and_hms_opt(0, 0, 0).unwrap().and_utc(),
    ))
}

fn make_timestamp(args: &[Literal]) -> KernelResult {
    arity("MAKE_TIMESTAMP", args, 6, 6)?;
    let n = "MAKE_TIMESTAMP";
    let date = ymd(n, int(n, args, 0)?, int(n, args, 1)?, int(n, args, 2)?)?;
    let hour = int(n, args, 3)?;
    let minute = int(n, args, 4)?;
    let seconds = num(n, args, 5)?;
    if !(0..24).contains(&hour) || !(0..60).contains(&minute) || !(0.0..60.0).contains(&seconds) {
        return Err(format!("{}: time field out of range", n));
    }
    let nanos = ((seconds - seconds.floor()) * 1e9).round() as u32;
    date.and_hms_nano_opt(hour as u32, minute as u32, seconds as u32, nanos)
        .map(|dt| Literal::Timestamp(dt.and_utc()))
        .ok_or_else(|| format!("{}: time field out of range", n))
}

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];
const DAYS: [&str; 7] = [
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
];

/// Parse `input` against a PostgreSQL pattern. Numeric fields take up to their
/// natural width of digits; names match case-insensitively by prefix; literal
/// characters are skipped leniently (any single non-digit character matches).
fn parse_with_pattern(name: &str, input: &str, fmt: &str) -> Result<DateTime<Utc>, KernelError> {
    let mut rest = input.trim();
    let (mut year, mut month, mut day) = (1970i64, 1i64, 1i64);
    let (mut hour, mut minute, mut second, mut nanos) = (0i64, 0i64, 0i64, 0u32);
    let mut pm: Option<bool> = None;
    let mut day_of_year: Option<i64> = None;

    let digits = |rest: &mut &str, width: usize| -> Result<i64, KernelError> {
        let n = rest
            .chars()
            .take(width)
            .take_while(|c| c.is_ascii_digit())
            .count();
        if n == 0 {
            return Err(format!(
                "{}: expected a number at '{}' for format '{}'",
                name, rest, fmt
            ));
        }
        let v = rest[..n].parse::<i64>().unwrap_or(0);
        *rest = &rest[n..];
        Ok(v)
    };

    for token in tokenize(fmt) {
        match token {
            Token::Literal(lit) => {
                for c in lit.chars() {
                    if rest.starts_with(c) {
                        rest = &rest[c.len_utf8()..];
                    } else if !c.is_ascii_digit() && !c.is_alphanumeric() {
                        // Lenient on punctuation: '2024/01/02' parses with 'YYYY-MM-DD'.
                        if let Some(first) = rest.chars().next().filter(|f| !f.is_alphanumeric()) {
                            rest = &rest[first.len_utf8()..];
                        }
                    }
                }
            }
            Token::Field { spelling, .. } => match spelling {
                "YYYY" => year = digits(&mut rest, 4)?,
                "YYY" => year = 2000 + digits(&mut rest, 3)?,
                "YY" => year = 2000 + digits(&mut rest, 2)?,
                "Y" => year = 2000 + digits(&mut rest, 1)?,
                "MM" => month = digits(&mut rest, 2)?,
                "DD" => day = digits(&mut rest, 2)?,
                "DDD" => day_of_year = Some(digits(&mut rest, 3)?),
                "HH24" => hour = digits(&mut rest, 2)?,
                "HH12" | "HH" => hour = digits(&mut rest, 2)?,
                "MI" => minute = digits(&mut rest, 2)?,
                "SS" => second = digits(&mut rest, 2)?,
                "MS" => nanos = digits(&mut rest, 3)? as u32 * 1_000_000,
                "US" => nanos = digits(&mut rest, 6)? as u32 * 1_000,
                "MONTH" | "Month" | "month" | "MON" | "Mon" | "mon" => {
                    month = 1 + take_name(name, &mut rest, &MONTHS, spelling.len() == 3)? as i64
                }
                "DAY" | "Day" | "day" | "DY" | "Dy" | "dy" => {
                    take_name(name, &mut rest, &DAYS, spelling.len() == 2)?;
                }
                "AM" | "PM" | "am" | "pm" => {
                    let lower = rest.get(..2).unwrap_or("").to_lowercase();
                    pm = Some(lower == "pm");
                    rest = rest.get(2..).unwrap_or("");
                }
                "A.M." | "P.M." | "a.m." | "p.m." => {
                    let lower = rest.get(..4).unwrap_or("").to_lowercase();
                    pm = Some(lower == "p.m.");
                    rest = rest.get(4..).unwrap_or("");
                }
                "TZ" | "OF" | "Q" | "WW" | "IW" | "J" | "D" => {
                    return Err(format!(
                        "{}: field '{}' cannot be used for parsing",
                        name, spelling
                    ))
                }
                _ => {}
            },
        }
    }

    if let Some(is_pm) = pm {
        hour = match (hour, is_pm) {
            (12, false) => 0,
            (h, true) if h < 12 => h + 12,
            (h, _) => h,
        };
    }
    let date = match day_of_year {
        Some(doy) => NaiveDate::from_yo_opt(year as i32, doy as u32)
            .ok_or_else(|| format!("{}: day of year {} is out of range", name, doy))?,
        None => ymd(name, year, month, day)?,
    };
    if !(0..24).contains(&hour) || !(0..60).contains(&minute) || !(0..61).contains(&second) {
        return Err(format!("{}: time field out of range in '{}'", name, input));
    }
    date.and_hms_nano_opt(hour as u32, minute as u32, second as u32, nanos)
        .map(|dt| dt.and_utc())
        .ok_or_else(|| format!("{}: '{}' is not a valid timestamp", name, input))
}

/// Consume a month or weekday name; returns its index.
fn take_name(
    name: &str,
    rest: &mut &str,
    names: &[&str],
    abbreviated: bool,
) -> Result<usize, KernelError> {
    let lower = rest.to_lowercase();
    for (i, candidate) in names.iter().enumerate() {
        let want = if abbreviated {
            &candidate[..3]
        } else {
            candidate
        };
        if lower.starts_with(want) {
            // A full name in the input still matches an abbreviated pattern.
            let consumed = if lower.starts_with(candidate) {
                candidate.len()
            } else {
                want.len()
            };
            *rest = &rest[consumed..];
            return Ok(i);
        }
    }
    Err(format!("{}: expected a name at '{}'", name, rest))
}
