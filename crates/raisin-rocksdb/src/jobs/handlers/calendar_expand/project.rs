//! Turning expanded instants into `raisin:Event` occurrence nodes.
//!
//! Everything here is deterministic: the same master and the same instant
//! always produce the same path, the same name and the same property map. That
//! is what makes the rebuild a DIFF rather than a delete-and-recreate — a
//! projection that churned node ids every 15 minutes would fill the revision
//! log and invalidate every cached query for no change at all.

use chrono::{DateTime, Datelike, Utc};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use std::collections::BTreeMap;

use super::master::{SeriesMaster, TYPE_OCCURRENCE};
use super::rule::Occurrence;
use super::{format_utc, EVENT_TYPE, OCCURRENCE_ROOT};

/// The projection subtree that belongs to one master.
///
/// Keyed by the master's NODE id, not its `__external_id`: the node id exists
/// for every master (including one created locally, which has no provider id
/// yet) and is unique within the workspace, so the subtree is both always
/// derivable and never shared between two series.
pub fn master_root(master_node_id: &str) -> String {
    format!("{OCCURRENCE_ROOT}/{master_node_id}")
}

/// Leaf name for one instance: the fixed-width UTC instant, compacted.
///
/// `20260331T070000Z` — sorts chronologically as a plain string, which is what
/// makes a `CHILD_OF` listing of one month come back in order for free.
pub fn occurrence_name(start_utc: DateTime<Utc>) -> String {
    start_utc.format("%Y%m%dT%H%M%SZ").to_string()
}

/// Full path of one generated occurrence.
///
/// The `YYYY/MM` levels are not decoration. There is no range pushdown on any
/// compound-index column in this engine, so a date-range query must be bounded
/// through the hierarchy; a month folder is the grouping level `CHILD_OF` can
/// use. They also keep a decade-long daily series off one parent's ordered-child
/// list.
pub fn occurrence_path(master_node_id: &str, start_utc: DateTime<Utc>) -> String {
    format!(
        "{}/{:04}/{:02}/{}",
        master_root(master_node_id),
        start_utc.year(),
        start_utc.month(),
        occurrence_name(start_utc)
    )
}

/// Build the property map of one occurrence node.
///
/// Returned as a sorted map so two builds of the same occurrence compare equal
/// regardless of iteration order — the diff below is a property-map comparison,
/// and an order-dependent one would rewrite every node on every rebuild.
pub fn occurrence_properties(
    master: &SeriesMaster,
    occ: &Occurrence,
) -> BTreeMap<String, PropertyValue> {
    let mut props: BTreeMap<String, PropertyValue> = BTreeMap::new();
    for (k, v) in &master.carried {
        props.insert(k.clone(), v.clone());
    }
    props.insert(
        "recurrence_type".into(),
        PropertyValue::String(TYPE_OCCURRENCE.into()),
    );
    props.insert(
        "start_utc".into(),
        PropertyValue::String(format_utc(occ.start_utc)),
    );
    props.insert(
        "end_utc".into(),
        PropertyValue::String(format_utc(occ.end_utc)),
    );
    props.insert(
        "start_local".into(),
        PropertyValue::String(occ.start_local.clone()),
    );
    props.insert(
        "end_local".into(),
        PropertyValue::String(occ.end_local.clone()),
    );
    // The slot this instance occupies. Identical to `start_utc` for a generated
    // occurrence — it exists so an exception and the occurrence it replaces are
    // joinable on ONE column, whichever of the two a query starts from.
    props.insert(
        "original_start_utc".into(),
        PropertyValue::String(format_utc(occ.start_utc)),
    );
    if let Some(ext) = &master.external_id {
        props.insert(
            "series_master_external_id".into(),
            PropertyValue::String(ext.clone()),
        );
    }
    // Deliberately NOT written: `recurrence` (an occurrence carrying its
    // master's rule would expand again), and every `__` reserved property. These
    // nodes are engine-derived, not mount-owned; claiming `__virtual` /
    // `__mount_id` would offer them to a mount's reconcile as its own.
    props
}

/// Build a whole occurrence node. `id` is supplied by the caller so an existing
/// node keeps its identity across rebuilds.
pub fn occurrence_node(
    id: String,
    workspace: &str,
    master: &SeriesMaster,
    occ: &Occurrence,
) -> Node {
    let path = occurrence_path(&master.node_id, occ.start_utc);
    let name = occurrence_name(occ.start_utc);
    Node {
        id,
        node_type: EVENT_TYPE.to_string(),
        name,
        path,
        workspace: Some(workspace.to_string()),
        properties: occurrence_properties(master, occ).into_iter().collect(),
        ..Default::default()
    }
}

/// Whether an existing occurrence node already matches what the projection
/// wants, so the rebuild can skip writing it.
///
/// Compares only the keys the projection OWNS. A property added to the node by
/// something else does not force a rewrite — but it also means the projection
/// never removes one, which is fine because nothing else writes these nodes.
pub fn matches(
    existing: &Node,
    desired: &std::collections::HashMap<String, PropertyValue>,
) -> bool {
    desired.iter().all(|(k, v)| {
        existing
            .properties
            .get(k)
            .is_some_and(|cur| prop_eq(cur, v))
    })
}

/// Property equality that survives the write path's own coercion.
///
/// An ISO-8601 string carrying a zone is stored as `PropertyValue::Date`, so the
/// `start_utc` the projection WRITES (a `String`) is never byte-equal to the
/// `start_utc` it READS BACK (a `Date`). Comparing them raw makes every
/// occurrence look changed on every rebuild — a full rewrite of the projection
/// every 15 minutes, forever, with `unchanged` permanently zero.
fn prop_eq(current: &PropertyValue, desired: &PropertyValue) -> bool {
    match (current, desired) {
        (PropertyValue::Date(ts), PropertyValue::String(s))
        | (PropertyValue::String(s), PropertyValue::Date(ts)) => {
            format_utc(*ts.as_datetime()) == *s
        }
        (a, b) => a == b,
    }
}
