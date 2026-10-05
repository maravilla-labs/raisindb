//! The `block_overlay_tombstones` cleanup runs by itself (plan Phase 11c,
//! owner request of 2026-10-04).
//!
//! Existing databases hold block overlays of deleted nodes that no delete
//! ever tombstoned. Reads already treat them as deleted (the read rule), so
//! this is cleanup, never the fix — but it is the kind nobody would trigger
//! by hand, so like the `node_path` backfill it queues itself: after the job
//! system starts, the branches whose `block_overlay_tombstones` state record
//! on THIS node is not `done` are cleaned up by ordinary `IndexRepair` jobs,
//! one branch at a time, through the same chain (`auto_node_path`):
//! streaming, bounded batches, resumable cursor, disk precheck and rate
//! limit, never on the boot path. A checkpoint ingest marks every branch
//! pending again and restarts the chain (a peer's checkpoint can carry
//! untombstoned versions). Nothing else re-runs it on a branch that reached
//! `done`: a delete materialization that later loses a race leaves its live
//! version stored (never served — the read rule ends it) until the admin
//! endpoint runs the cleanup or history GC drops that delete.
//!
//! `RAISIN_BLOCK_OVERLAY_TOMBSTONES_AUTO=0` (or `false`/`off`/`no`) turns the
//! automatic run off; the admin endpoint still works.

use super::auto_node_path::schedule_chain;
use super::branches::list_branches;
use super::enqueue::list_repositories;
use super::{load_state, repair_node_id, RepairKind};
use crate::RocksDBStorage;
use raisin_error::Result;
use std::sync::Arc;
use std::time::Duration;

/// The environment switch for the automatic cleanup (default ON).
pub const BLOCK_OVERLAY_AUTO_ENV: &str = "RAISIN_BLOCK_OVERLAY_TOMBSTONES_AUTO";

/// After the `node_path`, localized-name and `property_index` chains' first
/// links, so the four are not competing for the first minutes.
pub const START_DELAY: Duration = Duration::from_secs(90);

/// `(tenant, repo, branch)`.
type BranchKey = (String, String, String);

/// Whether the automatic cleanup is enabled (anything but `0`/`false`/`off`/
/// `no`, any case, or unset).
pub fn auto_enabled() -> bool {
    std::env::var(BLOCK_OVERLAY_AUTO_ENV)
        .map(|v| {
            !matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "off" | "no"
            )
        })
        .unwrap_or(true)
}

/// Every `(tenant, repo, branch)` whose cleanup has not reached `done` on
/// this node, in order (empty while the automatic cleanup is off).
pub fn pending_branches(storage: &RocksDBStorage) -> Result<Vec<BranchKey>> {
    if !auto_enabled() {
        return Ok(Vec::new());
    }
    let db = storage.db();
    let node_id = repair_node_id(storage);
    let slug = RepairKind::BlockOverlayTombstones.slug();
    let mut out = Vec::new();
    for (tenant_id, repo_id) in list_repositories(db)? {
        for branch in list_branches(db, &tenant_id, &repo_id)? {
            let state = load_state(db, &tenant_id, &repo_id, &branch, slug, &node_id)?;
            if state.is_none_or(|s| s.status != "done") {
                out.push((tenant_id.clone(), repo_id.clone(), branch));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Called once the job system runs: start the chain after [`START_DELAY`].
pub fn schedule_after_start(storage: Arc<RocksDBStorage>) {
    if !auto_enabled() {
        tracing::info!(
            env = BLOCK_OVERLAY_AUTO_ENV,
            "automatic block_overlay_tombstones cleanup is off"
        );
        return;
    }
    schedule_chain(storage, RepairKind::BlockOverlayTombstones, START_DELAY);
}
