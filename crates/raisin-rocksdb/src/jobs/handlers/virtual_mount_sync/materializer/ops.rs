//! The staged-operation queue: what a batch is made of, how duplicates collapse,
//! and how the byte budget is estimated.

use std::collections::HashMap;

use super::index::VirtualMeta;
use super::node_paths::join_path;
use crate::jobs::handlers::virtual_mount_sync::config::MappedNode;

/// One staged operation in a batch. Deletes and upserts share the queue so a
/// delta page that creates then deletes the same item applies in the order the
/// provider reported it.
// Deletes are much smaller than upserts, so the enum is sized by `Upsert`. Not
// worth boxing: a batch is overwhelmingly upserts (deletes only arrive from a
// delta tombstone or a reconcile pass), so the padding is rarely paid, and an
// allocation per operation is exactly the per-item cost this batching exists to
// remove.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum BatchOp {
    Upsert {
        rel_path: String,
        mapped: MappedNode,
        virt: VirtualMeta,
    },
    Delete {
        external_id: String,
    },
}

impl BatchOp {
    /// The external id this op targets, for order-preserving dedup.
    pub(super) fn external_id(&self) -> &str {
        match self {
            BatchOp::Upsert { virt, .. } => &virt.external_id,
            BatchOp::Delete { external_id } => external_id,
        }
    }
}

/// Outcome counts for one batch.
///
/// Deliberately NOT `Copy`: it carries [`Self::first_error`], because a count of
/// rejections with no reason attached is what let a mount report `OK · 100
/// failed` for hours. The message is the diagnosis; the number alone is not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BatchStats {
    pub written: usize,
    pub skipped: usize,
    pub deleted: usize,
    /// Items rejected individually (validation, RLS, a foreign node at the path)
    /// without aborting the batch.
    pub failed: usize,
    /// The FIRST item-level rejection seen, verbatim.
    ///
    /// First rather than last: every item of a mount whose target workspace
    /// forbids its node type fails the same way, so the first one is the whole
    /// story and the last is just the newest copy of it.
    pub first_error: Option<String>,
}

impl BatchStats {
    pub fn merge(&mut self, other: BatchStats) {
        self.written += other.written;
        self.skipped += other.skipped;
        self.deleted += other.deleted;
        self.failed += other.failed;
        if self.first_error.is_none() {
            self.first_error = other.first_error;
        }
    }
}

/// Collapse operations that target the same item or the same path, keeping the
/// LAST — a page's later entry is the newer state, and this is also what makes a
/// create-then-delete pair resolve to the delete.
///
/// Without this a duplicated `external_id` whose two occurrences resolve to
/// different paths produces TWO nodes claiming the same external id; the next
/// sync matches one arbitrarily and the other is orphaned forever, invisible to
/// reconcile because its external id IS in `seen`.
pub fn dedup_ops(ops: Vec<BatchOp>, mount_path: &str) -> Vec<BatchOp> {
    let mut keep_by_external: HashMap<&str, usize> = HashMap::new();
    for (i, op) in ops.iter().enumerate() {
        keep_by_external.insert(op.external_id(), i);
    }
    let mut keep_by_path: HashMap<String, usize> = HashMap::new();
    for (i, op) in ops.iter().enumerate() {
        if keep_by_external.get(op.external_id()) != Some(&i) {
            continue;
        }
        if let BatchOp::Upsert { rel_path, .. } = op {
            let path = join_path(mount_path, rel_path);
            if let Some(prev) = keep_by_path.insert(path.clone(), i) {
                tracing::warn!(
                    path = %path,
                    first = %ops[prev].external_id(),
                    second = %op.external_id(),
                    "two external items resolve to the same node path; keeping the last. \
                     Check the mount's path_template or mapping function — the losing item \
                     will be re-imported and overwrite this one on every sync."
                );
            }
        }
    }

    ops.iter()
        .enumerate()
        .filter(|(i, op)| {
            if keep_by_external.get(op.external_id()) != Some(i) {
                return false;
            }
            match op {
                BatchOp::Upsert { rel_path, .. } => {
                    keep_by_path.get(&join_path(mount_path, rel_path)) == Some(i)
                }
                BatchOp::Delete { .. } => true,
            }
        })
        .map(|(_, op)| op.clone())
        .collect()
}

/// Approximate serialized size of an item, for the batch byte budget. Mail and
/// document bodies dominate, so the property payload is what is measured.
pub fn estimate_op_bytes(op: &BatchOp) -> usize {
    match op {
        BatchOp::Delete { external_id } => external_id.len() + 64,
        BatchOp::Upsert {
            rel_path,
            mapped,
            virt,
        } => {
            let props: usize = mapped
                .properties
                .iter()
                .map(|(k, v)| k.len() + json_size(v))
                .sum();
            props
                + rel_path.len()
                + mapped.node_type.len()
                + mapped.name.as_deref().map_or(0, str::len)
                + virt.external_id.len()
                + 256 // node envelope: id, timestamps, reserved props
        }
    }
}

/// Cheap size estimate for a JSON value — no allocation, unlike re-serializing.
fn json_size(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Null => 4,
        serde_json::Value::Bool(_) => 5,
        serde_json::Value::Number(_) => 12,
        serde_json::Value::String(s) => s.len() + 2,
        serde_json::Value::Array(a) => 2 + a.iter().map(json_size).sum::<usize>(),
        serde_json::Value::Object(o) => {
            2 + o
                .iter()
                .map(|(k, v)| k.len() + 3 + json_size(v))
                .sum::<usize>()
        }
    }
}
