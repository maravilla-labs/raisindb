//! TO_CHAR(timestamp, format) and the PostgreSQL format-pattern tokenizer it
//! shares with TO_TIMESTAMP / TO_DATE.
//!
//! Supported patterns: YYYY YYY YY Y MM MON Mon mon MONTH Month month DD DDD
//! D DY Dy dy DAY Day day HH24 HH12 HH MI SS MS US AM PM am pm Q WW IW J TZ
//! OF, the `FM` prefix (suppress padding), and `"quoted"` text. Anything else
//! is copied through verbatim, as PostgreSQL does.

use super::super::args::{arity, text, timestamp};
use super::super::{Kernel, KernelCategory, KernelResult};
use crate::analyzer::Literal;
use chrono::{DateTime, Datelike, Timelike, Utc};

pub(super) const KERNEL: Kernel = Kernel {
    name: "TO_CHAR",
    aliases: &[],
    category: KernelCategory::Temporal,
    signature: "TO_CHAR(timestamp, format) -> TEXT",
    deterministic: true,
    strict: true,
    func: to_char,
};

fn to_char(args: &[Literal]) -> KernelResult {
    arity("TO_CHAR", args, 2, 2)?;
    let ts = timestamp("TO_CHAR", args, 0)?;
    let fmt = text("TO_CHAR", args, 1)?;
    Ok(Literal::Text(format_timestamp(ts, fmt)))
}

/// One piece of a format pattern.
#[derive(Debug, PartialEq)]
pub(super) enum Token<'a> {
    /// A recognised field, with the exact spelling the user wrote (case
    /// decides capitalisation of month and day names) and whether `FM`
    /// preceded it.
    Field { spelling: &'a str, fill_mode: bool },
    /// Text copied through unchanged.
    Literal(&'a str),
}

/// Longest-match first, so `HH24` wins over `HH`, `MONTH` over `MON`.
const FIELDS: &[&str] = &[
    "YYYY", "YYY", "YY", "Y", "MONTH", "Month", "month", "MON", "Mon", "mon", "MM", "MI", "MS",
    "DDD", "DD", "DAY", "Day", "day", "DY", "Dy", "dy", "D", "HH24", "HH12", "HH", "SS", "US",
    "AM", "PM", "am", "pm", "A.M.", "P.M.", "a.m.", "p.m.", "Q", "WW", "IW", "J", "TZ", "OF",
];

pub(super) fn tokenize(fmt: &str) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let mut rest = fmt;
    let mut fill_mode = false;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('"') {
            let end = after.find('"').unwrap_or(after.len());
            tokens.push(Token::Literal(&after[..end]));
            rest = after.get(end + 1..).unwrap_or("");
            continue;
        }
        if let Some(after) = rest.strip_prefix("FM") {
            fill_mode = true;
            rest = after;
            continue;
        }
        if let Some(field) = FIELDS.iter().find(|f| rest.starts_with(**f)) {
            tokens.push(Token::Field {
                spelling: &rest[..field.len()],
                fill_mode,
            });
            fill_mode = false;
            rest = &rest[field.len()..];
            continue;
        }
        let ch_len = rest.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        tokens.push(Token::Literal(&rest[..ch_len]));
        rest = &rest[ch_len..];
    }
    tokens
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// Apply the case of `spelling` (all upper / capitalised / all lower) to `name`.
fn cased(spelling: &str, name: &str) -> String {
    if spelling.chars().all(|c| c.is_uppercase()) {
        name.to_uppercase()
    } else if spelling.starts_with(|c: char| c.is_lowercase()) {
        name.to_lowercase()
    } else {
        name.to_string()
    }
}

/// Render `ts` with a PostgreSQL pattern.
pub(crate) fn format_timestamp(ts: DateTime<Utc>, fmt: &str) -> String {
    let mut out = String::with_capacity(fmt.len() + 8);
    for token in tokenize(fmt) {
        match token {
            Token::Literal(s) => out.push_str(s),
            Token::Field {
                spelling,
                fill_mode,
            } => out.push_str(&render_field(ts, spelling, fill_mode)),
        }
    }
    out
}

fn pad(value: impl std::fmt::Display, width: usize, fill_mode: bool) -> String {
    if fill_mode {
        value.to_string()
    } else {
        format!("{:0>width$}", value, width = width)
    }
}

/// PostgreSQL `WW`: week of the year, week 1 starting on January 1st.
fn week_of_year(ts: DateTime<Utc>) -> u32 {
    (ts.ordinal() - 1) / 7 + 1
}

fn render_field(ts: DateTime<Utc>, spelling: &str, fm: bool) -> String {
    let hour12 = match ts.hour() % 12 {
        0 => 12,
        h => h,
    };
    let month_name = MONTHS[(ts.month() - 1) as usize];
    let day_name = DAYS[ts.weekday().num_days_from_monday() as usize];
    match spelling {
        "YYYY" => pad(ts.year(), 4, fm),
        "YYY" => pad(ts.year().rem_euclid(1000), 3, fm),
        "YY" => pad(ts.year().rem_euclid(100), 2, fm),
        "Y" => pad(ts.year().rem_euclid(10), 1, fm),
        "MM" => pad(ts.month(), 2, fm),
        "MONTH" | "Month" | "month" => {
            let name = cased(spelling, month_name);
            if fm {
                name
            } else {
                format!("{:<9}", name)
            }
        }
        "MON" | "Mon" | "mon" => cased(spelling, &month_name[..3]),
        "DDD" => pad(ts.ordinal(), 3, fm),
        "DD" => pad(ts.day(), 2, fm),
        "D" => (ts.weekday().num_days_from_sunday() + 1).to_string(),
        "DAY" | "Day" | "day" => {
            let name = cased(spelling, day_name);
            if fm {
                name
            } else {
                format!("{:<9}", name)
            }
        }
        "DY" | "Dy" | "dy" => cased(spelling, &day_name[..3]),
        "HH24" => pad(ts.hour(), 2, fm),
        "HH12" | "HH" => pad(hour12, 2, fm),
        "MI" => pad(ts.minute(), 2, fm),
        "SS" => pad(ts.second(), 2, fm),
        "MS" => pad(ts.nanosecond() / 1_000_000, 3, fm),
        "US" => pad(ts.nanosecond() / 1_000, 6, fm),
        "AM" | "PM" => if ts.hour() < 12 { "AM" } else { "PM" }.to_string(),
        "am" | "pm" => if ts.hour() < 12 { "am" } else { "pm" }.to_string(),
        "A.M." | "P.M." => if ts.hour() < 12 { "A.M." } else { "P.M." }.to_string(),
        "a.m." | "p.m." => if ts.hour() < 12 { "a.m." } else { "p.m." }.to_string(),
        "Q" => ((ts.month() - 1) / 3 + 1).to_string(),
        "WW" => pad(week_of_year(ts), 2, fm),
        "IW" => pad(ts.iso_week().week(), 2, fm),
        "J" => (ts.timestamp().div_euclid(86_400) + 2_440_588).to_string(),
        "TZ" => "UTC".to_string(),
        "OF" => "+00".to_string(),
        other => other.to_string(),
    }
}
