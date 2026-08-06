//! Recurrence expansion: the derived occurrence projection.
//!
//! # Why this exists
//!
//! With series masters canonical, `WHERE start_utc >= ? AND start_utc < ?`
//! misses every recurring event: the master's `start_utc` is its FIRST
//! occurrence, which for a weekly standing meeting is usually years outside the
//! window being asked about. That is already the case for Microsoft Graph
//! (which collapses `calendarView` occurrences back to the master) and became
//! the case for Google when Stage 5b stopped passing `singleEvents=true`. It is
//! a regression in date-range querying, not a missing nicety.
//!
//! # What it is, and what it is not
//!
//! The projection is **derived, never authoritative**. The authoritative record
//! is the series master plus its exception nodes; occurrence nodes are a
//! materialized view of the expansion, rebuilt on a rolling window by a periodic
//! job, and safe to delete wholesale at any time. Two consequences run through
//! the whole module:
//!
//! * The write path must never touch one — see [`guard`].
//! * The rebuild is a DIFF, not a rewrite. Paths and property maps are
//!   deterministic ([`project`]), so a rebuild that finds nothing changed writes
//!   nothing and costs no revisions.
//!
//! # Layout
//!
//! ```text
//! /_occurrences/{master_node_id}/{YYYY}/{MM}/{YYYYMMDDTHHMMSSZ}
//! ```
//!
//! Outside every mount subtree on purpose. A mount's full reconcile prunes
//! anything under its own path that the provider did not report, so a projection
//! materialized inside a mount would be deleted by the next full sync and
//! rebuilt by the next tick, forever.

mod guard;
mod handler;
mod job;
mod master;
mod project;
mod rebuild;
mod rule;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_fixtures;
#[cfg(test)]
mod tests_guard;
#[cfg(test)]
mod tests_project;
#[cfg(test)]
mod tests_rebuild;

use chrono::{DateTime, Utc};

pub use guard::{is_derived_occurrence, node_refusal, path_refusal};
pub use handler::CalendarExpandHandler;
pub use job::{run_occurrence_rebuild, RebuildSummary};

/// Root of the derived projection, workspace-relative.
///
/// Leading underscore so it sorts and reads as engine-owned. It is NOT
/// configurable: two deployments disagreeing about where the projection lives
/// would each treat the other's as ordinary user content, and the write guard
/// keys off this prefix.
pub const OCCURRENCE_ROOT: &str = "/_occurrences";

/// Node type the projection writes. The same type as the master, distinguished
/// by `recurrence_type` — a separate type would need its own copy of every
/// calendar query.
pub const EVENT_TYPE: &str = "raisin:Event";

/// Fixed-width UTC form. Fixed width is load-bearing: it is what makes
/// lexicographic order equal chronological order on the indexed `start_utc`
/// column, which is what a `PropertyOrderScan` over it relies on.
pub const UTC_FMT: &str = "%Y-%m-%dT%H:%M:%SZ";
/// Wall-clock form for `start_local` / `end_local`, no offset.
pub const LOCAL_FMT: &str = "%Y-%m-%dT%H:%M:%S";
/// All-day wall-clock form.
pub const LOCAL_DATE_FMT: &str = "%Y-%m-%d";

/// How far back the rolling window reaches. Past occurrences stay queryable for
/// a month so "what did I do last week" works without a rebuild-on-demand.
pub const WINDOW_PAST_DAYS: i64 = 30;
/// How far forward. A little over a year, so an annual series always has its
/// next instance projected.
pub const WINDOW_FUTURE_DAYS: i64 = 400;

/// Hard cap on instances generated from ONE master, per rebuild.
///
/// `FREQ=DAILY` with no `COUNT`/`UNTIL` is an infinite set, and the window alone
/// does not bound `FREQ=MINUTELY`. 1600 covers a daily series across the full
/// 430-day window four times over; anything denser is reported as truncated
/// rather than materialized.
pub const MAX_OCCURRENCES_PER_MASTER: u16 = 1600;

/// Minimum interval between two rebuilds of the same workspace.
///
/// The scheduler ticks every 60s; expanding every workspace's whole calendar
/// that often would be pure waste, since the window only moves by a day at a
/// time. Enforced twice — a process-local throttle and the cluster lease's TTL —
/// because job dedup in this engine is per-PROCESS, so on an N-node cluster the
/// process throttle alone lets N nodes rebuild in parallel.
pub const REBUILD_INTERVAL_SECS: u64 = 900;

/// Render an instant in the projection's canonical UTC form.
pub fn format_utc(at: DateTime<Utc>) -> String {
    at.format(UTC_FMT).to_string()
}
