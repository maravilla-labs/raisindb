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
    Some(SeriesMaster {
        node_id: node.id.clone(),
        external_id: str_prop(node, "__external_id"),
        recurrence: recurrence_lines(node),
        timezone: str_prop(node, "timezone"),
        start_utc: instant_prop(node, "start_utc"),
        end_utc: instant_prop(node, "end_utc"),
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
