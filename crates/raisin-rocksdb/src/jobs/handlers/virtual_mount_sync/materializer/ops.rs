//! The staged-operation queue: what a batch is made of, how duplicates collapse,
//! and how the byte budget is estimated.

use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

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
        /// Ordinary (non-reserved) property values to LAND on the node as part
        /// of the same read-modify-write — the merged values a conflict
        /// resolver produced.
        ///
        /// The one exception to "a stamp touches `__`-prefixed keys only", and
        /// it has to be in the same op rather than a second write: the merged
        /// values and the `__pushed_state` recording them as pushed must land
        /// together or the node converges against a baseline it does not hold.
        /// Two writes could also interleave with a user edit between them.
        /// `None` on every stamp but a merge.
        merged: Option<Map<String, Value>>,
        /// Whether this stamp ADOPTS the node — writes `__virtual`,
        /// `__mount_id` and `__external_id` onto a node the mount did not
        /// previously own, because the engine has just created its remote
        /// counterpart.
        ///
        /// A typed flag rather than three entries in `merged`, because `merged`
        /// drops every `__`-prefixed key on purpose: that map comes from a
        /// conflict resolver, and a resolver able to forge `__mount_id` could
        /// hand one mount's node to another, or fabricate provenance that the
        /// delete rails then treat as proof of ownership. Adoption is the
        /// engine's own assertion about a call it just made, so it travels as a
        /// flag the engine sets and a mapper cannot reach. `mount_id` is not
        /// carried: it comes from the scope, so an adopt cannot name a mount
        /// other than the one whose drain is running.
        adopt: bool,
        /// Set when this stamp RE-KEYS the node: the provider answered the
        /// update with an external id different from the one the engine sent,
        /// and this is the id the node had before. `external_id` above is the
        /// new one.
        ///
        /// For a key-addressed store the key IS the identity — an S3 rename is
        /// a copy to a new key — so an update that renames leaves the engine
        /// holding an id that resolves to nothing. Every later run would then
        /// re-import the object as a new node and reconcile the old one away,
        /// losing its history and its local edits.
        ///
        /// A typed field rather than "just stamp the new id", because a re-key
        /// is not a metadata amendment: it rewrites `__external_id`, the
        /// property the delete rails and the whole index read as the node's
        /// provider identity. Carrying the PREVIOUS id lets the index drop its
        /// old entry in the same batch, so the rest of the run looks the node
        /// up under the id it now has.
        ///
        /// SUBORDINATE nodes are not carried along: a child's external id
        /// embeds its parent's (see `child_external_id`), so re-keying a parent
        /// that has children would strand them under the old prefix. No adapter
        /// both emits children and re-keys — the child shape is mail
        /// attachments, the re-key shape is key-addressed blob stores — so this
        /// is a documented limit rather than a partial implementation.
        rekey: Option<String>,
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
    /// The external ids of the rejected items — WHICH ones, not just how many.
    ///
    /// A count is not actionable and, worse, is not recoverable: the read paths
    /// persisted their cursor past a rejected item and the change was never
    /// re-delivered, so the item stayed permanently stale or absent while the
    /// run reported `ok`. Carrying the ids is what lets the caller park them,
    /// retry them, and refuse to call the run clean while any are outstanding.
    ///
    /// Bounded by [`MAX_FAILED_IDS`]: a mount failing wholesale is already
    /// caught by the failure budget, and an unbounded list would put a whole
    /// mailbox into mount state.
    pub failed_ids: Vec<String>,
    /// Mount-owned nodes the full reconcile deliberately did NOT delete because
    /// their path is excluded by the mount's filters.
    ///
    /// Reported rather than merely skipped: adding an `exclude` pattern to a
    /// live mount leaves already-synced nodes behind, unmanaged, and an operator
    /// cannot otherwise tell that from "the mount deleted them" or from "the
    /// pattern did nothing". Never a `failed` — nothing was rejected.
    pub retained_excluded: usize,
}

/// How many rejected ids one batch carries. Past this the failure is systemic,
/// not per-item, and `check_failure_budget` is the mechanism that applies.
pub const MAX_FAILED_IDS: usize = 100;

impl BatchStats {
    /// Record an item-level rejection: the count, the reason, and the id.
    pub fn note_failure(&mut self, external_id: &str, error: &str) {
        self.failed += 1;
        if self.first_error.is_none() {
            self.first_error = Some(error.to_string());
        }
        if self.failed_ids.len() < MAX_FAILED_IDS
            && !self.failed_ids.iter().any(|id| id == external_id)
        {
            self.failed_ids.push(external_id.to_string());
        }
    }

    pub fn merge(&mut self, other: BatchStats) {
        self.written += other.written;
        self.skipped += other.skipped;
        self.deleted += other.deleted;
        self.stamped += other.stamped;
        self.failed += other.failed;
        self.retained_excluded += other.retained_excluded;
        if self.first_error.is_none() {
            self.first_error = other.first_error;
        }
        for id in other.failed_ids {
            if self.failed_ids.len() >= MAX_FAILED_IDS {
                break;
            }
            if !self.failed_ids.iter().any(|existing| *existing == id) {
                self.failed_ids.push(id);
            }
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
    // Two DIFFERENT items landing on one path is not a duplicate — it is two
    // pieces of real content whose names happen to collide, and dropping either
    // loses it.
    //
    // Dropping the loser is what this used to do, with a WARN that counted as
    // neither a write nor a failure. The consequences compounded: the dropped
    // item was already in the walk's `seen` set, so reconcile saw nothing wrong;
    // the next run etag-skipped the survivor and staged the loser instead, which
    // resolved to the same path and rewrote the node with the other item's
    // content; and the run after that swapped them back. One node alternating
    // between two messages forever, one revision and one trigger fan-out per
    // run, reported as `written: 2, failed: 0`.
    //
    // Both are kept instead, with the colliding paths disambiguated by a short
    // digest of each item's own external id. That is stable per item and
    // independent of page order, so the two nodes settle immediately rather than
    // trading places. The mount is still misconfigured — a `path_template` that
    // is not unique, or a mapper naming children after a filename — and the WARN
    // still says so.
    let mut path_owners: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, op) in ops.iter().enumerate() {
        if keep_by_external.get(op.external_id()) != Some(&i) {
            continue;
        }
        if let BatchOp::Upsert { rel_path, .. } = op {
            path_owners
                .entry(join_path(mount_path, rel_path))
                .or_default()
                .push(i);
        }
    }
    let mut disambiguate: HashSet<usize> = HashSet::new();
    for (path, owners) in &path_owners {
        if owners.len() < 2 {
            continue;
        }
        tracing::warn!(
            path = %path,
            colliding = owners.len(),
            ids = ?owners.iter().map(|i| ops[*i].external_id()).collect::<Vec<_>>(),
            "several external items resolve to the same node path; each is being given a \
             distinct suffix so none is lost. Check the mount's path_template or mapping \
             function — a node path must be unique per item."
        );
        disambiguate.extend(owners.iter().copied());
    }

    // The keep decision is frozen into flags so `ops` can be CONSUMED below.
    // `keep_by_external` borrows each op's external id, and that borrow was the
    // only reason every survivor — mapped node payload and all — had to be
    // cloned out, so a flush carrying mail bodies paid a full deep copy of the
    // batch just so one rare branch could rewrite one path string. One copy per
    // op becomes none; the disambiguated op is edited in place instead.
    let mut keep = vec![false; ops.len()];
    for &i in keep_by_external.values() {
        keep[i] = true;
    }
    drop(keep_by_external);

    ops.into_iter()
        .enumerate()
        .filter(|(i, _)| keep[*i])
        .map(|(i, mut op)| {
            if let BatchOp::Upsert { rel_path, virt, .. } = &mut op {
                if disambiguate.contains(&i) {
                    *rel_path = suffix_path(rel_path, &virt.external_id);
                }
            }
            op
        })
        .collect()
}

/// Append a short, stable digest of `external_id` to the last segment of
/// `rel_path`, before any extension, so two items that collide on a name become
/// two distinct paths that do not move between runs.
fn suffix_path(rel_path: &str, external_id: &str) -> String {
    let digest = short_digest(external_id);
    match rel_path.rfind('.') {
        // Only a real extension on the FINAL segment; a dot inside a directory
        // name is not one.
        Some(dot) if dot > rel_path.rfind('/').map_or(0, |s| s + 1) => {
            format!("{}-{}{}", &rel_path[..dot], digest, &rel_path[dot..])
        }
        _ => format!("{rel_path}-{digest}"),
    }
}

/// FNV-1a, truncated to 8 hex chars. Not cryptographic and does not need to be:
/// it only has to separate two names inside one mount, deterministically.
fn short_digest(value: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    format!("{:08x}", (hash >> 32) as u32)
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
            merged,
            node_bytes,
            ..
        } => {
            node_bytes
                + node_id.len()
                + external_id.len()
                + pushed_state
                    .as_ref()
                    .map_or(0, |m| json_size(&Value::Object(m.clone())))
                + merged
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
