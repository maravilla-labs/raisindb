//! The `property_index` rebuild runs by itself (plan Phase 7b, owner decision
//! of 2026-10-04: `index.skip_unchanged` is ON by default).
//!
//! The flag says the operator wants skip-unchanged writes; a branch's
//! `property_index` rebuild being `done` UNDER THIS NODE'S ID says they are
//! safe there (the rebuild fills the holes full re-puts used to heal by
//! accident). That per-branch gate stays. What changes is who runs the
//! rebuild: with the flag on, nobody has to — the branches whose rebuild is
//! not `done` on this node are rebuilt by ordinary `IndexRepair` jobs on the
//! unified job queue, through the same one-branch-at-a-time chain as the
//! `node_path` backfill (`auto_node_path`): streaming, bounded batches,
//! resumable cursor, disk precheck and rate limit, never on the boot path.
//!
//! - **At start** ([`schedule_after_start`]): the chain starts after
//!   [`START_DELAY`], once the job system runs.
//! - **On request** ([`request_rebuild`]): a branch created (a fork starts
//!   with no record, so its writes are full puts until rebuilt), a merge from
//!   an unrebuilt source, a checkpoint ingest (every branch invalidated). The
//!   requested branch is queued directly when it has no live job, and the
//!   chain is started for everything else pending (a no-op while it runs).
//!
//! Until a branch is rebuilt its writes are the full puts they always were,
//! so a rebuild that is late, failed (no disk headroom, say) or switched off
//! costs write amplification, never correctness.
//! `RAISIN_PROPERTY_INDEX_AUTO_REBUILD=0` (or `false`/`off`/`no`) turns the
//! automatic rebuild off; the admin endpoint still works.

use super::auto_node_path::{schedule_chain, start_chain};
use super::auto_targets::enqueue_branch;
use super::branches::list_branches;
use super::enqueue::list_repositories;
use super::requests::{debounced, registered_storage_for, BranchKey};
use super::{property_index_rebuilt, repair_node_id, RepairKind};
use crate::RocksDBStorage;
use raisin_error::Result;
use rocksdb::DB;
use std::sync::Arc;
use std::time::Duration;

/// The environment switch for the automatic rebuild (default ON).
pub const PROPERTY_INDEX_AUTO_REBUILD_ENV: &str = "RAISIN_PROPERTY_INDEX_AUTO_REBUILD";

/// How long after the job system starts the chain begins — after the
/// `node_path` backfill's and the localized name build's first links, so the
/// three are not competing for the first minute.
pub const START_DELAY: Duration = Duration::from_secs(60);

/// Whether this node rebuilds by itself: `index.skip_unchanged` is on (a
/// rebuild unlocks nothing otherwise) and the switch is not off.
pub fn auto_rebuild_enabled(storage: &RocksDBStorage) -> bool {
    storage.nodes_impl().index_skip_unchanged()
        && std::env::var(PROPERTY_INDEX_AUTO_REBUILD_ENV)
            .map(|v| {
                !matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "0" | "false" | "off" | "no"
                )
            })
            .unwrap_or(true)
}

/// Every `(tenant, repo, branch)` whose `property_index` rebuild is not
/// `done` on this node, in order (empty while the automatic rebuild is off).
pub fn pending_property_index_branches(storage: &RocksDBStorage) -> Result<Vec<BranchKey>> {
    if !auto_rebuild_enabled(storage) {
        return Ok(Vec::new());
    }
    let db = storage.db();
    let node_id = repair_node_id(storage);
    let mut out = Vec::new();
    for (tenant_id, repo_id) in list_repositories(db)? {
        for branch in list_branches(db, &tenant_id, &repo_id)? {
            if !property_index_rebuilt(db, &tenant_id, &repo_id, &branch, &node_id) {
                out.push((tenant_id.clone(), repo_id.clone(), branch));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Called once the job system runs: start the chain after [`START_DELAY`].
pub fn schedule_after_start(storage: Arc<RocksDBStorage>) {
    if !auto_rebuild_enabled(&storage) {
        tracing::info!(
            env = PROPERTY_INDEX_AUTO_REBUILD_ENV,
            skip_unchanged = storage.nodes_impl().index_skip_unchanged(),
            "automatic property_index rebuild is off"
        );
        return;
    }
    schedule_chain(storage, RepairKind::PropertyIndex, START_DELAY);
}

/// Ask for the rebuild of `(tenant, repo, branch)` on the storage over `db`
/// (`branch` `None`: whatever is pending), in the background. Never blocks
/// and never fails the caller; without a running job system over `db` it does
/// nothing (the next start picks the branch up).
pub fn request_rebuild(db: &DB, tenant_id: &str, repo_id: &str, branch: Option<&str>) {
    let Some(storage) = registered_storage_for(db) else {
        return;
    };
    if !auto_rebuild_enabled(&storage) {
        return;
    }
    let target = (
        tenant_id.to_string(),
        repo_id.to_string(),
        branch.unwrap_or_default().to_string(),
    );
    if debounced(RepairKind::PropertyIndex, &target) {
        return;
    }
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    handle.spawn(async move {
        let kind = RepairKind::PropertyIndex;
        let (t, r, b) = (&target.0, &target.1, &target.2);
        let direct = if b.is_empty() {
            Ok(0)
        } else {
            enqueue_branch(&storage, kind, t, r, b).await
        };
        let chained = start_chain(&storage, kind).await;
        match (direct, chained) {
            (Ok(0), Ok(0)) => {}
            (Ok(_), Ok(_)) => tracing::debug!(?target, "property_index rebuild queued on request"),
            (Err(e), _) | (_, Err(e)) => {
                tracing::warn!(error = %e, "could not queue a property_index rebuild")
            }
        }
    });
}
