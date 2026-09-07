//! Temporal kernels and the two helpers every temporal reader shares:
//! [`parse_timestamp`] (text → instant) and [`format_interval`] (duration →
//! PostgreSQL text).
//!
//! # Time zone
//! RaisinDB stores and computes every instant in UTC. `DATE_TRUNC`,
//! `EXTRACT`, `TO_CHAR` and `CURRENT_DATE` all evaluate in UTC; there is no
//! session time zone and no `AT TIME ZONE` yet.
//!
//! # DATE
//! There is no separate DATE type in the analyzer. A date is a TIMESTAMPTZ at
//! 00:00:00 UTC, which is what `CURRENT_DATE`, `TO_DATE`, `MAKE_DATE` and
//! `CAST(x AS DATE)` produce. That keeps date arithmetic (`CURRENT_DATE -
//! INTERVAL '7 days'`) and comparisons against `created_at` working without a
//! second temporal type.

mod age;
mod convert;
mod part;
mod to_char;
mod trunc;

use super::args::arity;
use super::{Kernel, KernelCategory, KernelResult};
use crate::analyzer::Literal;
use chrono::{DateTime, Duration, Utc};

macro_rules! volatile {
    ($name:expr, $aliases:expr, $sig:expr, $f:expr) => {
        Kernel {
            name: $name,
            aliases: $aliases,
            category: KernelCategory::Temporal,
            signature: $sig,
            deterministic: false,
            strict: true,
            func: $f,
        }
    };
}

pub(super) static KERNELS: &[Kernel] = &[
    volatile!(
        "CURRENT_TIMESTAMP",
        &[
            "LOCALTIMESTAMP",
            "TRANSACTION_TIMESTAMP",
            "STATEMENT_TIMESTAMP",
            "CLOCK_TIMESTAMP"
        ],
        "CURRENT_TIMESTAMP -> TIMESTAMPTZ",
        current_timestamp
    ),
    volatile!(
        "CURRENT_DATE",
        &[],
        "CURRENT_DATE -> TIMESTAMPTZ (midnight UTC)",
        current_date
    ),
    volatile!(
        "CURRENT_TIME",
        &["LOCALTIME"],
        "CURRENT_TIME -> TEXT (HH:MM:SS.ffffff+00)",
        current_time
    ),
    trunc::KERNEL,
    part::KERNEL,
    age::KERNEL,
    convert::TO_TIMESTAMP,
    convert::TO_DATE,
    convert::MAKE_DATE,
    convert::MAKE_TIMESTAMP,
    to_char::KERNEL,
];

fn current_timestamp(args: &[Literal]) -> KernelResult {
    arity("CURRENT_TIMESTAMP", args, 0, 0)?;
    Ok(Literal::Timestamp(Utc::now()))
}

fn current_date(args: &[Literal]) -> KernelResult {
    arity("CURRENT_DATE", args, 0, 0)?;
    Ok(Literal::Timestamp(midnight(Utc::now())))
}

fn current_time(args: &[Literal]) -> KernelResult {
    arity("CURRENT_TIME", args, 0, 0)?;
    Ok(Literal::Text(
        Utc::now().format("%H:%M:%S%.6f+00").to_string(),
    ))
}

/// The instant at 00:00:00 UTC on `ts`'s calendar day.
pub(crate) fn midnight(ts: DateTime<Utc>) -> DateTime<Utc> {
    ts.date_naive()
        .and_hms_opt(0, 0, 0)
        .map(|n| n.and_utc())
        .unwrap_or(ts)
}

/// Parse a timestamp string in the formats RaisinDB accepts: RFC 3339, ISO
/// 8601 without zone (taken as UTC), the same with a space instead of `T`,
/// with or without fractional seconds, and a bare `YYYY-MM-DD` (midnight UTC).
pub fn parse_timestamp(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    for fmt in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M",
    ] {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
            return Some(dt.and_utc());
        }
    }
    // `2024-01-15 10:30:00+02` — PostgreSQL's own output shape.
    for fmt in ["%Y-%m-%d %H:%M:%S%.f%#z", "%Y-%m-%d %H:%M:%S%#z"] {
        if let Ok(dt) = DateTime::parse_from_str(s, fmt) {
            return Some(dt.with_timezone(&Utc));
        }
    }
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|n| n.and_utc())
}

/// Render a duration the way PostgreSQL prints an interval:
/// `3 days 04:05:06`, `-1 days -02:00:00`, `00:00:00.5`, `1 day`.
///
/// Intervals here are exact durations (a month was already converted to 30
/// days when the literal was parsed), so there is never a `mon` component.
pub fn format_interval(d: &Duration) -> String {
    let negative = *d < Duration::zero();
    let abs = if negative { -*d } else { *d };
    let total_micros = abs.num_microseconds().unwrap_or(i64::MAX);
    let days = total_micros / 86_400_000_000;
    let rem = total_micros % 86_400_000_000;
    let hours = rem / 3_600_000_000;
    let minutes = (rem % 3_600_000_000) / 60_000_000;
    let seconds = (rem % 60_000_000) / 1_000_000;
    let micros = rem % 1_000_000;

    let sign = if negative { "-" } else { "" };
    let mut parts = Vec::new();
    if days != 0 {
        parts.push(format!(
            "{}{} day{}",
            sign,
            days,
            if days == 1 { "" } else { "s" }
        ));
    }
    if rem != 0 || parts.is_empty() {
        let mut time = format!("{}{:02}:{:02}:{:02}", sign, hours, minutes, seconds);
        if micros != 0 {
            time.push_str(&format!(".{:06}", micros));
            while time.ends_with('0') {
                time.pop();
            }
        }
        parts.push(time);
    }
    parts.join(" ")
}
