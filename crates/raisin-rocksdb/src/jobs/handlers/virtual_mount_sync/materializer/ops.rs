//! The staged-operation queue: what a batch is made of, how duplicates collapse,
//! and how the byte budget is estimated.

use serde_json::{Map, Value};
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
    /// Re-stamp ONLY the engine-owned reserved properties of a node that is
    /// already there — the write drain's stamp-back after a successful push.
    ///
    /// Deliberately not an `Upsert`. An upsert rebuilds the property map from
    /// mapper output, so using one here would need a full re-map of a node the
    /// engine has no fresh remote item for, and any property the mapper does
    /// not reproduce — a user's own edit, a field another system wrote — would
    /// be silently dropped as the price of recording an etag. This variant is a
    /// read-modify-write that touches `__`-prefixed keys and nothing else.
    StampVirtual {
        node_id: String,
        external_id: String,
        /// Provider etag as of the push. `None` leaves the stored one alone —
        /// an adapter that returns no etag must not be read as "no etag".
        etag: Option<String>,
        synced_at: String,
        /// Watched-field values as actually pushed. `None` leaves the stored
        /// map alone (a stamp that only refreshes the etag).
        pushed_state: Option<Map<String, Value>>,
        /// Serialized size of the node being stamped, measured by the drain
        /// from the node it already read (see [`estimate_node_bytes`]).
        ///
        /// Carried explicitly because the stamp is a read-modify-write that
        /// re-writes the node WHOLE — body and all — so its replication cost is
        /// the node's size, not the size of the metadata being amended. Without
        /// it the batch byte budget cannot see a drain at all.
        node_bytes: usize,
    },
}

impl BatchOp {
    /// The external id this op targets, for order-preserving dedup.
    pub(super) fn external_id(&self) -> &str {
        match self {
            BatchOp::Upsert { virt, .. } => &virt.external_id,
            BatchOp::Delete { external_id } => external_id,
            BatchOp::StampVirtual { external_id, .. } => external_id,
        }
    }

    /// Whether this op re-states the node WHOLE (upsert, delete) rather than
    /// amending its reserved metadata.
    fn is_authoritative(&self) -> bool {
        !matches!(self, BatchOp::StampVirtual { .. })
    }
}

/// Outcome counts for one batch.
///
/// Deliberately NOT `Copy`: it carries [`Self::first_error`], because a count of
/// rejections with no reason attached is what let a mount report `OK · 100
/// failed` for hours. The message is the diagnosis; the number alone is not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BatchStats {
    /// Items materialized from provider data — an upsert that actually landed.
    ///
    /// Deliberately NOT incremented by a [`BatchOp::StampVirtual`]. Two readers
    /// depend on that: the console's per-run `written` count, which an operator
    /// reads as "items imported", and
    /// [`SyncBatcher::check_failure_budget`](crate::jobs::handlers::virtual_mount_sync::batch)
    /// — whose whole condition is `written == 0`, i.e. "nothing this mount tried
    /// to import could be written". A drain that stamps its own metadata is not
    /// evidence that the target workspace accepts the mapper's node type, so
    /// counting stamps here disarmed the wholesale-rejection guard for the rest
    /// of the run and turned a fail-fast `misconfigured` into a 600s timeout
    /// reported as OK.
    pub written: usize,
    pub skipped: usize,
    pub deleted: usize,
    /// Reserved-metadata stamps applied to nodes that were already there (the
    /// write drain's stamp-back). Counted separately from [`Self::written`];
    /// see that field for why.
    pub stamped: usize,
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
        self.stamped += other.stamped;
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
///
/// [`BatchOp::StampVirtual`] does NOT simply take part in "last wins". An
/// upsert or a delete re-states the node whole, so it supersedes a stamp for
/// the same item whichever side of it the stamp fell on: keeping a later stamp
/// would drop the upsert entirely (a stamp writes no mapper output), and
/// keeping an earlier one would re-apply metadata the upsert has already
/// rewritten. A stamp therefore survives only when it is the only kind of op
/// that item has in this batch — which is the ordinary case, because the write
/// drain flushes ahead of the read phases.
pub fn dedup_ops(ops: Vec<BatchOp>, mount_path: &str) -> Vec<BatchOp> {
    let mut keep_by_external: HashMap<&str, usize> = HashMap::new();
    for (i, op) in ops.iter().enumerate() {
        let authoritative = op.is_authoritative();
        match keep_by_external.get(op.external_id()) {
            // An authoritative op never loses to a stamp, in either order.
            Some(&prev) if !authoritative && ops[prev].is_authoritative() => continue,
            _ => {
                keep_by_external.insert(op.external_id(), i);
            }
        }
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
                BatchOp::Delete { .. } | BatchOp::StampVirtual { .. } => true,
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
        // A stamp REWRITES THE WHOLE NODE (`stage_stamp` → `upsert_node`), so
        // it costs that node's full size in the replication record, not the
        // size of the `__`-prefixed keys it amends. Charging it as a small item
        // was not a harmless under-count: `state_only` turned on over an
        // existing mailbox makes every node diverge, so a drain stages up to
        // `max_items_per_sync` stamps in one pass, and at ~1 KB
        // apiece none of them ever reaches the 4 MiB budget — all of them land
        // in ONE transaction, i.e. one `ApplyRevision` holding a full snapshot
        // of every mail body in the drain. `node_bytes` is measured by the
        // drain from the node it already read.
        BatchOp::StampVirtual {
            node_id,
            external_id,
            pushed_state,
            node_bytes,
            ..
        } => {
            node_bytes
                + node_id.len()
                + external_id.len()
                + pushed_state
                    .as_ref()
                    .map_or(0, |m| json_size(&Value::Object(m.clone())))
                + 256
        }
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

/// Fallback charge for a node whose size could not be measured. Deliberately
/// large: an unknown node must not be treated as free, or the failure mode is
/// the oversized-record one the budget exists to prevent.
const UNKNOWN_NODE_BYTES: usize = 64 * 1024;

/// Serialized size of a node, for charging a [`BatchOp::StampVirtual`] against
/// the batch byte budget.
///
/// Measured by serializing, unlike [`estimate_op_bytes`]'s allocation-free
/// walk: the drain calls this once per node it actually pushes or seeds, next
/// to a provider round-trip and a transactional write, so the allocation is
/// noise — and a stamp's cost is the whole node, which is exactly the thing a
/// property-map walk of the amendment cannot see.
pub fn estimate_node_bytes(node: &raisin_models::nodes::Node) -> usize {
    serde_json::to_vec(node).map_or(UNKNOWN_NODE_BYTES, |v| v.len())
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
