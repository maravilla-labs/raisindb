//! Reading a `raisin:Event` series master out of a node, and nothing else.
//!
//! Kept pure and node-shaped (no storage, no async) so the whole expansion can
//! be property-tested without a database.

use chrono::{DateTime, Utc};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;

/// `recurrence_type` values this module cares about. They are the enum declared
/// in `raisin_event.yaml`; anything else is treated as a non-recurring event.
pub const TYPE_SERIES_MASTER: &str = "series_master";
pub const TYPE_OCCURRENCE: &str = "occurrence";
pub const TYPE_EXCEPTION: &str = "exception";

/// The subset of a `raisin:Event` series master the expander needs.
#[derive(Debug, Clone)]
pub struct SeriesMaster {
    pub node_id: String,
    /// Provider id, from the `__external_id` reserved property. `None` for a
    /// master that was never synced; the projection still works, because the
    /// authoritative master link is the projection PATH, not this field.
    pub external_id: Option<String>,
    /// RFC 5545 content lines, verbatim from the `recurrence` array.
    pub recurrence: Vec<String>,
    /// IANA zone of `start_local` / `end_local`. `None` means the master is
    /// interpreted in UTC — correct only for a rule that cannot cross a DST
    /// boundary, which is why a missing zone is reported rather than assumed.
    pub timezone: Option<String>,
    pub start_utc: Option<DateTime<Utc>>,
    pub end_utc: Option<DateTime<Utc>>,
    /// Naive wall-clock start (`YYYY-MM-DDTHH:MM:SS`, or `YYYY-MM-DD` all-day).
    pub start_local: Option<String>,
    pub all_day: bool,
    /// RFC 5545 STATUS. A cancelled master expands to nothing.
    pub status: Option<String>,
    /// Properties carried onto every generated occurrence verbatim.
    pub carried: Vec<(String, PropertyValue)>,
}

/// Properties an occurrence inherits from its master unchanged.
///
/// Deliberately an allow-list rather than "everything except the time fields":
/// a new property on `raisin:Event` must be considered before it starts being
/// duplicated across a year of occurrences, and `recurrence` in particular must
/// NEVER be copied — an occurrence carrying its master's RRULE would expand
/// again on the next rebuild.
pub const CARRIED_PROPS: &[&str] = &[
    "title",
    "ical_uid",
    "calendar_id",
    "timezone",
    "all_day",
    "status",
    "show_as",
    "my_response",
    "organizer_email",
    "organizer_name",
    "attendees",
    "location",
    "location_geo",
    "online_meeting_url",
    "url",
    "description_html",
    "description_text",
];

fn str_prop(node: &Node, key: &str) -> Option<String> {
    match node.properties.get(key) {
        Some(PropertyValue::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

/// A timestamp-valued property, as a UTC instant.
///
/// Reads BOTH representations on purpose. `start_utc` is declared `type: String`
/// and every mapper writes a string — but the write path coerces an ISO-8601
/// string that carries a zone into `PropertyValue::Date`, so the value that
/// comes back out is a `Date` while the value that went in was a `String`. A
/// reader matching only on `String` sees nothing at all, silently, which is
/// exactly how this module first failed to expand a single master.
pub(super) fn instant_prop(node: &Node, key: &str) -> Option<DateTime<Utc>> {
    match node.properties.get(key) {
        Some(PropertyValue::Date(ts)) => Some(*ts.as_datetime()),
        Some(PropertyValue::String(s)) => parse_utc(s),
        _ => None,
    }
}

fn bool_prop(node: &Node, key: &str) -> Option<bool> {
    match node.properties.get(key) {
        Some(PropertyValue::Boolean(b)) => Some(*b),
        _ => None,
    }
}

fn parse_utc(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

/// The UTC instant of a naive wall clock in an IANA zone.
///
/// THIS IS THE OTHER HALF OF A CONTRACT THE ADAPTERS RELY ON. Microsoft Graph
/// reports a naive local datetime plus a separate `timeZone` and never a UTC
/// instant, so the ms-graph mapper deliberately writes `start_utc: null` and
/// records that the instant is "derived later where a real tz database exists".
/// This is that place — and until it existed, every Outlook series master
/// failed expansion with `master has no parsable start_utc` and simply had no
/// occurrences (320 of them in one production run), while the mount reported a
/// clean sync.
///
/// Local time is not a bijection, so both irregular cases are decided here
/// rather than left to a panic-on-unwrap:
///
/// * AMBIGUOUS (the DST fall-back hour, which happens twice): take the EARLIER
///   instant. That is the first time the clock reads e.g. 02:30, and it is what
///   Outlook, Google Calendar and RFC 5545 implementations do.
/// * NONEXISTENT (the spring-forward gap, which never happens): step forward in
///   one-minute increments to the first instant that does exist — the same
///   "clock skips ahead" behaviour a person would apply. Bounded at the gap's
///   plausible maximum so a pathological zone cannot spin.
fn instant_in_zone(local: &str, tz_name: &str) -> Option<DateTime<Utc>> {
    use chrono::offset::LocalResult;
    use chrono::{NaiveDate, NaiveDateTime, TimeZone};

    let tz: chrono_tz::Tz = tz_name.parse().ok()?;

    // Both shapes the mapper writes: a wall clock, or a bare date for all-day.
    let naive = NaiveDateTime::parse_from_str(local, super::LOCAL_FMT)
        .ok()
        .or_else(|| {
            NaiveDate::parse_from_str(local, super::LOCAL_DATE_FMT)
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
        })?;

    match tz.from_local_datetime(&naive) {
        LocalResult::Single(dt) => Some(dt.with_timezone(&Utc)),
        LocalResult::Ambiguous(earlier, _later) => Some(earlier.with_timezone(&Utc)),
        LocalResult::None => {
            // Longest DST jump in the tz database is 2h; 180 minutes is slack.
            for minutes in 1..=180 {
                let shifted = naive + chrono::Duration::minutes(minutes);
                if let LocalResult::Single(dt) = tz.from_local_datetime(&shifted) {
                    return Some(dt.with_timezone(&Utc));
                }
            }
            None
        }
    }
}

/// A timestamp property, falling back to its local twin resolved in `timezone`.
///
/// The stored UTC column WINS whenever it is present — it is the indexed,
/// fixed-width value everything else orders by, and `build_ical` re-derives the
/// wall clock from it for exactly that reason. The fallback only fills a hole.
fn instant_or_local(
    node: &Node,
    utc_key: &str,
    local_key: &str,
    timezone: Option<&str>,
) -> Option<DateTime<Utc>> {
    if let Some(instant) = instant_prop(node, utc_key) {
        return Some(instant);
    }
    let local = str_prop(node, local_key)?;
    instant_in_zone(&local, timezone?)
}

/// The `recurrence` array as content lines. Absent, empty, or non-string
/// elements answer an empty vector rather than an error: a master with no rule
/// is simply not expandable, which the caller reports once.
fn recurrence_lines(node: &Node) -> Vec<String> {
    match node.properties.get("recurrence") {
        Some(PropertyValue::Array(items)) => items
            .iter()
            .filter_map(|v| match v {
                PropertyValue::String(s) => Some(s.trim().to_string()),
                _ => None,
            })
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

/// `recurrence_type` of a node, lowercased. Absent answers `"single"`, matching
/// the nodetype's documented default.
pub fn recurrence_type(node: &Node) -> String {
    str_prop(node, "recurrence_type")
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "single".to_string())
}

/// Parse a node as a series master. `None` for anything that is not one.
pub fn parse_master(node: &Node) -> Option<SeriesMaster> {
    if recurrence_type(node) != TYPE_SERIES_MASTER {
        return None;
    }
    let carried = CARRIED_PROPS
        .iter()
        .filter_map(|k| node.properties.get(*k).map(|v| (k.to_string(), v.clone())))
        .collect();
    // Resolved ONCE here rather than at each use, so `build_ical`, the duration
    // and the projection all see one coherent master — a provider that reports
    // only local time must not produce a master that expands but has
    // zero-length occurrences.
    let timezone = str_prop(node, "timezone");
    Some(SeriesMaster {
        node_id: node.id.clone(),
        external_id: str_prop(node, "__external_id"),
        recurrence: recurrence_lines(node),
        start_utc: instant_or_local(node, "start_utc", "start_local", timezone.as_deref()),
        end_utc: instant_or_local(node, "end_utc", "end_local", timezone.as_deref()),
        timezone,
        start_local: str_prop(node, "start_local"),
        all_day: bool_prop(node, "all_day").unwrap_or(false),
        status: str_prop(node, "status"),
        carried,
    })
}

/// One exception node's identity: which series it overrides and which slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExceptionSlot {
    pub series_master_external_id: String,
    /// `original_start_utc`, normalized to the projection's fixed-width UTC
    /// form so it compares byte-equal with a generated occurrence's key.
    pub original_start_utc: String,
}

/// Parse a node as an exception. `None` unless it declares both halves of its
/// identity — an exception missing either cannot be matched to a slot, so it
/// cannot suppress one and must not silently suppress the wrong one.
pub fn parse_exception(node: &Node) -> Option<ExceptionSlot> {
    if recurrence_type(node) != TYPE_EXCEPTION {
        return None;
    }
    let master = str_prop(node, "series_master_external_id")?;
    let normalized = instant_prop(node, "original_start_utc").map(super::format_utc)?;
    Some(ExceptionSlot {
        series_master_external_id: master,
        original_start_utc: normalized,
    })
}
