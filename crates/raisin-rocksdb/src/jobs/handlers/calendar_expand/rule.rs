//! RFC 5545 expansion: a series master plus a window in, concrete instants out.
//!
//! Two properties this module exists to guarantee:
//!
//! 1. **Expansion is always window-bounded AND count-bounded.** `FREQ=DAILY`
//!    with no `COUNT` and no `UNTIL` is an infinite set. Every call passes a
//!    half-open `[start, end)` window and a hard cap, and a truncated result is
//!    reported rather than silently short.
//! 2. **Timezone correctness is the whole point.** "every Tuesday 09:00
//!    Europe/Zurich" is 08:00Z while CET is in force and 07:00Z under CEST. The
//!    rule is therefore evaluated in the master's IANA zone — never by adding a
//!    fixed offset to a UTC instant, which is the implementation this module is
//!    here to prevent.

use chrono::{DateTime, Duration, Utc};
use rrule::{RRuleSet, Tz};
use std::str::FromStr;

use super::master::SeriesMaster;
use super::{LOCAL_DATE_FMT, LOCAL_FMT};

/// One generated instance of a series.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    pub start_utc: DateTime<Utc>,
    pub end_utc: DateTime<Utc>,
    /// Wall clock in the master's zone, no offset. `YYYY-MM-DD` when all-day.
    pub start_local: String,
    pub end_local: String,
}

/// Why a master could not be expanded. Every variant is an operator-visible
/// data problem, never a transient one, so the job logs and moves on.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExpandError {
    #[error("master has no recurrence lines")]
    NoRule,
    #[error("master has no parsable start_utc")]
    NoStart,
    #[error("timezone '{0}' is not a known IANA zone")]
    BadTimezone(String),
    #[error("recurrence rule rejected: {0}")]
    BadRule(String),
}

/// The outcome of expanding one master.
#[derive(Debug, Clone)]
pub struct Expansion {
    pub occurrences: Vec<Occurrence>,
    /// The cap was reached, so the window is NOT fully covered. Surfaced so a
    /// short projection is a logged fact rather than a silently missing week.
    pub truncated: bool,
}

/// Content-line prefixes the recurrence array may legally carry.
const KNOWN_PREFIXES: &[&str] = &["RRULE", "EXRULE", "RDATE", "EXDATE"];

/// Normalize one stored recurrence line into an iCalendar content line.
///
/// Providers are not consistent: Google emits full content lines
/// (`RRULE:FREQ=WEEKLY`), while a hand-authored or converted value can arrive as
/// a bare rule body (`FREQ=WEEKLY`). A bare body is promoted to `RRULE:`.
/// A `DTSTART` line is DROPPED — the master's own `start_local`/`timezone` is
/// the authority, and honouring an embedded one would let the two disagree.
fn normalize_line(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let head = line
        .split([':', ';'])
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_uppercase();
    if head == "DTSTART" {
        return None;
    }
    if KNOWN_PREFIXES.contains(&head.as_str()) {
        return Some(line.to_string());
    }
    // A bare rule body: no recognized property name at all.
    Some(format!("RRULE:{line}"))
}

/// Build the iCalendar block the `rrule` crate parses.
///
/// `DTSTART` is emitted with `TZID` when the master has a zone, so the crate
/// evaluates the rule in local time and hands back correctly-offset instants
/// across a DST transition. Without a zone it falls back to the UTC form, which
/// is right for a UTC-anchored series and wrong for a local one — hence
/// `timezone` being mandatory on a recurring event in the nodetype docs.
pub fn build_ical(master: &SeriesMaster) -> Result<String, ExpandError> {
    let lines: Vec<String> = master
        .recurrence
        .iter()
        .filter_map(|l| normalize_line(l))
        .collect();
    if lines.is_empty() {
        return Err(ExpandError::NoRule);
    }
    let start_utc = master.start_utc.ok_or(ExpandError::NoStart)?;

    let dtstart = match &master.timezone {
        Some(tz_name) => {
            let tz: chrono_tz::Tz = tz_name
                .parse()
                .map_err(|_| ExpandError::BadTimezone(tz_name.clone()))?;
            // Derive the wall clock from the UTC instant rather than trusting
            // `start_local`: the two are written by the same mapper, but only
            // `start_utc` is the indexed, fixed-width column everything else
            // orders by, so it is the one that must win if they ever disagree.
            let local = start_utc.with_timezone(&tz);
            format!("DTSTART;TZID={tz_name}:{}", local.format("%Y%m%dT%H%M%S"))
        }
        None => format!("DTSTART:{}", start_utc.format("%Y%m%dT%H%M%SZ")),
    };

    let mut out = String::with_capacity(dtstart.len() + 64);
    out.push_str(&dtstart);
    for line in lines {
        out.push('\n');
        out.push_str(&line);
    }
    Ok(out)
}

/// Expand `master` over the half-open window `[window_start, window_end)`.
///
/// `limit` caps the number of instances produced. It is not a tuning knob: it is
/// what stops an infinite rule from being an unbounded allocation.
pub fn expand(
    master: &SeriesMaster,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
    limit: u16,
) -> Result<Expansion, ExpandError> {
    let ical = build_ical(master)?;
    let set = RRuleSet::from_str(&ical).map_err(|e| ExpandError::BadRule(e.to_string()))?;

    // Duration is carried from the master, not recomputed per instance. An
    // occurrence that straddles a DST transition therefore keeps its ELAPSED
    // length (a 1h meeting stays 1h), which is what both providers do; the
    // wall-clock end moves by an hour, which the local strings show.
    let duration = match (master.start_utc, master.end_utc) {
        (Some(s), Some(e)) if e >= s => e - s,
        _ => Duration::zero(),
    };

    let after = window_start.with_timezone(&Tz::UTC);
    let before = window_end.with_timezone(&Tz::UTC);
    let result = set.after(after).before(before).all(limit);

    let tz: Option<chrono_tz::Tz> = match &master.timezone {
        Some(name) => Some(
            name.parse()
                .map_err(|_| ExpandError::BadTimezone(name.clone()))?,
        ),
        None => None,
    };

    let occurrences = result
        .dates
        .into_iter()
        .map(|d| {
            let start = d.with_timezone(&Utc);
            let end = start + duration;
            Occurrence {
                start_local: local_string(start, tz, master.all_day),
                end_local: local_string(end, tz, master.all_day),
                start_utc: start,
                end_utc: end,
            }
        })
        .collect();

    Ok(Expansion {
        occurrences,
        truncated: result.limited,
    })
}

/// Render an instant as the wall clock a user sees, in the master's zone.
fn local_string(at: DateTime<Utc>, tz: Option<chrono_tz::Tz>, all_day: bool) -> String {
    let fmt = if all_day { LOCAL_DATE_FMT } else { LOCAL_FMT };
    match tz {
        Some(tz) => at.with_timezone(&tz).format(fmt).to_string(),
        None => at.format(fmt).to_string(),
    }
}
