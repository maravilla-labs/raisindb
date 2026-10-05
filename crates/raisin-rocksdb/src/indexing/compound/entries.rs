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

/// The system timestamps the write layer stamps on every node version
/// (`Node::ensure_write_timestamps`). A version WITHOUT one is legacy data
/// (written before the stamping fix), not a modeling choice.
const SYSTEM_ORDER_COLUMNS: [&str; 2] = ["__created_at", "__updated_at"];

/// Whether `node` belongs in `def` but has no entry there: every leading
/// (equality) column derives, yet the trailing ORDER column is a system
/// timestamp the version lacks.
///
/// [`compound_entries`] skips such a node, which is right for a leading
/// column (a node with no value cannot match the equality the planner seeks)
/// and WRONG for the order column: `CHILD_OF(p) ORDER BY created_at` must
/// return a child with a NULL `created_at` — the row scan does — and an
/// index without it answers short, with no residual left to notice. A build
/// that meets one counts it ([`super::build::BuildOutcome::unindexable`]) and
/// never stamps `Ready`, so the planner keeps scanning (fail closed).
pub fn unrepresentable(def: &CompoundIndexDefinition, node: &Node) -> bool {
    if !def.has_order_column {
        return false;
    }
    let Some((order, leading)) = def.columns.split_last() else {
        return false;
    };
    if !SYSTEM_ORDER_COLUMNS.contains(&order.property.as_str()) {
        return false;
    }
    let value = |column: &raisin_models::nodes::properties::schema::CompoundIndexColumn| {
        crate::repositories::NodeRepositoryImpl::extract_compound_column_value(
            node,
            &column.property,
            &column.column_type,
        )
    };
    leading.iter().all(|column| value(column).is_some()) && value(order).is_none()
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
