//! The one rule the write path must never break: **a derived occurrence is not
//! writable.**
//!
//! The authoritative record of a recurring event is its series master plus its
//! exception nodes. Occurrence nodes are a projection — the rebuild deletes and
//! recreates them whenever the master changes — so an edit to one is lost at the
//! next tick with no error anywhere. That silent loss is the failure this module
//! exists to make impossible, which is why the check lives in TWO places: a path
//! filter in the drain's candidate selection, so a projection node is never even
//! nominated, and a property check on the node the drain actually re-reads, so a
//! projection node materialized somewhere unexpected is still refused out loud.

use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;

use super::master::TYPE_OCCURRENCE;
use super::OCCURRENCE_ROOT;

/// The refusal message. Deliberately says what to do instead — a bare
/// "not writable" leaves an operator with an edit they cannot land and no idea
/// where it belongs.
fn message(what: &str) -> String {
    format!(
        "{what} is a derived recurrence occurrence and is not writable. It is regenerated \
         from its series master on every rebuild, so an edit here is discarded. Edit the \
         series master to change the whole series, or create an exception node \
         (recurrence_type: exception) carrying series_master_external_id and \
         original_start_utc to change this single instance."
    )
}

/// Whether a path lies inside the derived-occurrence projection.
///
/// Matches the root itself and anything under it, and — this is the part that
/// matters — does NOT match a sibling that merely shares the prefix, so a real
/// mount at `/_occurrences_archive` is untouched.
pub fn is_projection_path(path: &str) -> bool {
    path == OCCURRENCE_ROOT || path.starts_with(&format!("{OCCURRENCE_ROOT}/"))
}

/// Refuse a write to a node identified by PATH alone.
///
/// Used by the drain's candidate selection, which works from the sync index and
/// has paths but not properties.
pub fn path_refusal(path: &str) -> Option<String> {
    is_projection_path(path).then(|| message(path))
}

/// Whether a node is a derived occurrence, by its own declaration.
pub fn is_derived_occurrence(node: &Node) -> bool {
    if is_projection_path(&node.path) {
        return true;
    }
    matches!(
        node.properties.get("recurrence_type"),
        Some(PropertyValue::String(s)) if s.trim().eq_ignore_ascii_case(TYPE_OCCURRENCE)
    )
}

/// Refuse a write to a node whose properties are in hand.
pub fn node_refusal(node: &Node) -> Option<String> {
    is_derived_occurrence(node).then(|| message(&node.path))
}
