//! The `node_delete_index` backfill runs by itself.
//!
//! Every database written before `NODE_DELETES` existed holds tombstones
//! without entries, and nobody would run a backfill by hand for a read-path
//! speedup — so, like the other upgrade repairs, it queues itself: after the
//! job system starts, the branches whose index is not `Ready` are backfilled
//! by ordinary `IndexRepair` jobs, one branch at a time, through the shared
//! chain (`repair::auto_node_path`), never on the boot path. A checkpoint
//! ingest (every record `NotBuilt`), a branch copy into a branch, and a
//! branch created here or by replication request it again.
//!
//! Until a branch is `Ready` its reads walk `NODES` as they always did:
//! correct, only slower.
//!
//! `RAISIN_NODE_DELETE_INDEX_AUTO=0` (or `false`/`off`/`no`) turns the
//! automatic run off; the admin endpoint still works. Entries are written by
//! every delete regardless, so a branch made `Ready` stays correct.

use crate::management::async_indexing::repair::{
    debounced, enqueue_branch, list_branches, list_repositories, registered_storage_for,
    schedule_chain, start_chain, RepairKind,
};
use crate::RocksDBStorage;
use raisin_error::Result;
use rocksdb::DB;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// The environment switch for the automatic backfill (default ON).
pub const NODE_DELETE_INDEX_AUTO_ENV: &str = "RAISIN_NODE_DELETE_INDEX_AUTO";

/// After the other upgrade chains' first links (node_path 30 s, localized
/// names 45 s, block overlays 90 s, timestamps 100 s).
pub const START_DELAY: Duration = Duration::from_secs(110);

/// `(tenant, repo, branch)`.
type BranchKey = (String, String, String);

static OVERRIDE: AtomicU8 = AtomicU8::new(0);

/// A process-wide TEST override of [`NODE_DELETE_INDEX_AUTO_ENV`] (`None`:
/// read the environment).
pub fn override_auto(enabled: Option<bool>) {
    let value = match enabled {
        None => 0,
        Some(true) => 1,
        Some(false) => 2,
    };
    OVERRIDE.store(value, Ordering::Relaxed);
}

/// Whether the automatic backfill is enabled.
pub fn auto_enabled() -> bool {
    match OVERRIDE.load(Ordering::Relaxed) {
        1 => return true,
        2 => return false,
        _ => {}
    }
    std::env::var(NODE_DELETE_INDEX_AUTO_ENV)
        .map(|v| {
            !matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "off" | "no"
            )
        })
        .unwrap_or(true)
}

/// Every `(tenant, repo, branch)` whose index is not `Ready`, in order
/// (empty while the automatic backfill is off).
pub fn pending_branches(storage: &RocksDBStorage) -> Result<Vec<BranchKey>> {
    if !auto_enabled() {
        return Ok(Vec::new());
    }
    let db = storage.db();
    let mut out = Vec::new();
    for (tenant_id, repo_id) in list_repositories(db)? {
        for branch in list_branches(db, &tenant_id, &repo_id)? {
            if !super::is_ready(db, &tenant_id, &repo_id, &branch) {
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
            env = NODE_DELETE_INDEX_AUTO_ENV,
            "automatic node_delete_index backfill is off"
        );
        return;
    }
    schedule_chain(storage, RepairKind::NodeDeleteIndex, START_DELAY);
}

/// Ask for the backfill of `(tenant, repo, branch)` on the storage over `db`,
/// in the background. Never blocks and never fails the caller; without a
/// running job system over `db` it does nothing (the next start picks the
/// branch up).
pub fn request_backfill(db: &DB, tenant_id: &str, repo_id: &str, branch: &str) {
    if !auto_enabled() {
        return;
    }
    let Some(storage) = registered_storage_for(db) else {
        return;
    };
    let target = (
        tenant_id.to_string(),
        repo_id.to_string(),
        branch.to_string(),
    );
    if debounced(RepairKind::NodeDeleteIndex, &target) {
        return;
    }
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    handle.spawn(async move {
        let kind = RepairKind::NodeDeleteIndex;
        let (t, r, b) = (&target.0, &target.1, &target.2);
        let direct = enqueue_branch(&storage, kind, t, r, b).await;
        let chained = start_chain(&storage, kind).await;
        match (direct, chained) {
            (Ok(0), Ok(0)) => {}
            (Ok(_), Ok(_)) => tracing::debug!(?target, "node_delete_index backfill queued"),
            (Err(e), _) | (_, Err(e)) => {
                tracing::warn!(error = %e, "could not queue a node_delete_index backfill")
            }
        }
    });
}

/// After a checkpoint ingest (the ingestor has marked every record
/// `NotBuilt`): restart the chain (a no-op while one runs).
pub async fn restart_after_ingest(storage: &RocksDBStorage) -> Result<usize> {
    if !auto_enabled() {
        return Ok(0);
    }
    start_chain(storage, RepairKind::NodeDeleteIndex).await
}
