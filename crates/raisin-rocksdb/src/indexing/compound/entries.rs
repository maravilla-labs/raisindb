//! The ONE derivation of a node version's COMPOUND_INDEX entries, and the one
//! grammar for reading an entry key back.
//!
//! An entry is identified by its GROUP: everything in the key before the
//! revision — `{t}\0{r}\0{b}\0{ws}\0cidx[_pub]\0{index}\0{col…}\0`. The full
//! key is `group ++ ~revision(16) ++ \0 ++ node_id`, so the writer, the
//! tombstoner, the rebuilds and the reader all agree byte-for-byte on which
//! `(tuple, node)` an entry belongs to. Diffs between versions are taken over
//! groups, never over `PropertyValue`s.

use crate::indexing::IndexCtx;
use crate::keys;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::schema::CompoundIndexDefinition;
use raisin_models::nodes::Node;
use std::collections::BTreeSet;

/// One compound entry of a node version: its group prefix (see module doc).
pub type CompoundGroup = Vec<u8>;

/// Every compound entry `node` indexes under `defs` (its type's resolved
/// declarations, plus its workspace's own — the callers in this module chain
/// them). An index whose columns the node cannot fill is skipped — the
/// writer's long-standing rule (a node without the column is simply not in
/// that index).
pub fn compound_entries<'d>(
    defs: impl IntoIterator<Item = &'d CompoundIndexDefinition>,
    ctx: &IndexCtx<'_>,
    node: &Node,
) -> BTreeSet<CompoundGroup> {
    let published = node.published_at.is_some();
    let mut out = BTreeSet::new();
    for def in defs {
        let mut values = Vec::with_capacity(def.columns.len());
        for column in &def.columns {
            match crate::repositories::NodeRepositoryImpl::extract_compound_column_value(
                node,
                &column.property,
                &column.column_type,
            ) {
                Some(value) => values.push(value),
                None => break,
            }
        }
        if values.len() != def.columns.len() {
            tracing::trace!(
                index = %def.name,
                node_id = %node.id,
                "node not in compound index: a column has no value"
            );
            continue;
        }
        out.insert(keys::compound_index_prefix(
            ctx.tenant_id,
            ctx.repo_id,
            ctx.branch,
            ctx.workspace,
            &def.name,
            &values,
            published,
        ));
    }
    out
}

/// Whether `node` belongs in `def` but is omitted from it in a way a query
/// can observe: its FIRST column derives (so a query binding it would reach
/// this node) yet a later column — a middle one, or the trailing ORDER column
/// — has no value (an absent property, a value of the wrong type, a system
/// timestamp a legacy version lacks).
///
/// [`compound_entries`] skips such a node, which is only right for a query
/// that binds every column: a null never equals anything. The planner also
/// serves a PREFIX (`status = x`, or `status = x ORDER BY due`) and a range
/// on the order column from the same index, and those must return the node —
/// the row scan does — so an index without it answers short, with no residual
/// left to notice. A build that meets one counts it
/// ([`super::build::BuildOutcome::unindexable`]) and never stamps `Ready`, and
/// the live writer marks a `Ready` index stale (`stage_stale_for_unfillable`),
/// so the planner keeps scanning (fail closed). A node whose FIRST column is
/// missing cannot be reached by any query that uses the index, and stays out.
pub fn unrepresentable(def: &CompoundIndexDefinition, node: &Node) -> bool {
    let Some((first, rest)) = def.columns.split_first() else {
        return false;
    };
    let value = |column: &raisin_models::nodes::properties::schema::CompoundIndexColumn| {
        crate::repositories::NodeRepositoryImpl::extract_compound_column_value(
            node,
            &column.property,
            &column.column_type,
        )
    };
    value(first).is_some() && rest.iter().any(|column| value(column).is_none())
}

/// The key of `group`'s entry for `node_id` at `revision`.
pub fn entry_key(group: &[u8], revision: &HLC, node_id: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(group.len() + 17 + node_id.len());
    key.extend_from_slice(group);
    key.extend_from_slice(&revision.encode_descending());
    key.push(0);
    key.extend_from_slice(node_id.as_bytes());
    key
}

/// A compound entry key read back: `(group, revision, node_id)`.
///
/// Parsed from the RIGHT: the node id is everything after the last `\0`
/// (ids are null-free) and the revision is the fixed 16 bytes before it.
/// Splitting the whole key on `\0` is wrong — an Integer, Boolean or timestamp
/// column and the descending revision are raw bytes that may contain `\0`.
pub fn parse_entry_key(key: &[u8]) -> Option<(&[u8], HLC, &str)> {
    let sep = key.iter().rposition(|b| *b == 0)?;
    let node_id = std::str::from_utf8(&key[sep + 1..]).ok()?;
    if node_id.is_empty() || sep < 17 {
        return None;
    }
    let rev_start = sep - 16;
    let revision = HLC::decode_descending(&key[rev_start..sep]).ok()?;
    if key[rev_start - 1] != 0 {
        return None;
    }
    Some((&key[..rev_start], revision, node_id))
}

/// The trailing 8 bytes of a group's last column, read as a big-endian `i64`
/// — the ORDER BY timestamp the scan reports (`CompoundIndexScanEntry`).
pub fn trailing_i64(group: &[u8]) -> Option<i64> {
    // group = `…\0{last column}\0`
    let body = group.strip_suffix(&[0])?;
    if body.len() < 9 || body[body.len() - 9] != 0 {
        return None;
    }
    let bytes: [u8; 8] = body[body.len() - 8..].try_into().ok()?;
    Some(i64::from_be_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::nodes::properties::PropertyValue;
    use std::collections::HashMap;

    fn def(order_type: &str, order_property: &str) -> CompoundIndexDefinition {
        serde_json::from_value(serde_json::json!({
            "name": "by_status_due",
            "columns": [
                { "property": "status", "column_type": "String" },
                { "property": order_property, "column_type": order_type }
            ],
            "has_order_column": true
        }))
        .unwrap()
    }

    fn node(extra: Option<(&str, PropertyValue)>) -> Node {
        let mut properties =
            HashMap::from([("status".to_string(), PropertyValue::String("open".into()))]);
        if let Some((k, v)) = extra {
            properties.insert(k.to_string(), v);
        }
        Node {
            id: "n1".into(),
            name: "n1".into(),
            path: "/n1".into(),
            node_type: "t:Doc".into(),
            properties,
            ..Default::default()
        }
    }

    const CTX: IndexCtx<'static> = IndexCtx {
        tenant_id: "t",
        repo_id: "r",
        branch: "main",
        workspace: "ws",
    };

    #[test]
    fn property_timestamp_order_column_is_indexed_for_date_and_rfc3339_string() {
        let d = def("Timestamp", "due_at");
        for value in [
            PropertyValue::String("2026-10-15T21:08:38Z".into()),
            PropertyValue::Date(
                chrono::DateTime::parse_from_rfc3339("2026-10-15T21:08:38Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc)
                    .into(),
            ),
        ] {
            let n = node(Some(("due_at", value)));
            assert_eq!(compound_entries([&d], &CTX, &n).len(), 1);
            assert!(!unrepresentable(&d, &n));
        }
    }

    #[test]
    fn property_timestamp_order_column_that_cannot_be_filled_is_unrepresentable() {
        let d = def("Timestamp", "due_at");
        // Absent, or holding no instant: skipped by the writer, and the
        // build must count it so the index never goes Ready (fail closed).
        for n in [
            node(None),
            node(Some(("due_at", PropertyValue::String("soon".into())))),
        ] {
            assert!(compound_entries([&d], &CTX, &n).is_empty());
            assert!(unrepresentable(&d, &n));
        }
    }

    #[test]
    fn a_missing_leading_column_is_still_just_not_in_the_index() {
        let d = def("Timestamp", "due_at");
        let mut n = node(None);
        n.properties.clear();
        assert!(!unrepresentable(&d, &n));
    }

    #[test]
    fn any_column_type_that_cannot_be_filled_after_the_first_is_unrepresentable() {
        // A String order column (or a middle column) behaves the same: a
        // prefix query served by the index would miss the node.
        assert!(unrepresentable(&def("String", "label"), &node(None)));
        let three: CompoundIndexDefinition = serde_json::from_value(serde_json::json!({
            "name": "abc",
            "columns": [
                { "property": "status", "column_type": "String" },
                { "property": "mid", "column_type": "String" },
                { "property": "last", "column_type": "String" }
            ],
            "has_order_column": false
        }))
        .unwrap();
        let only_last = node(Some(("last", PropertyValue::String("x".into()))));
        assert!(unrepresentable(&three, &only_last));
        let all = {
            let mut n = only_last.clone();
            n.properties
                .insert("mid".into(), PropertyValue::String("y".into()));
            n
        };
        assert!(!unrepresentable(&three, &all));
    }

    #[test]
    fn entries_for_later_instants_sort_before_earlier_ones() {
        let d = def("Timestamp", "due_at");
        let at = |s: &str| {
            compound_entries(
                [&d],
                &CTX,
                &node(Some(("due_at", PropertyValue::String(s.into())))),
            )
            .into_iter()
            .next()
            .unwrap()
        };
        assert!(at("2026-06-01T00:00:00Z") < at("2026-01-01T00:00:00Z"));
    }
}
