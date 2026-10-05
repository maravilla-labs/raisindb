//! The `node_path` backfill runs by itself (plan Phase 10b, owner decision of
//! 2026-10-03).
//!
//! Every other repair is admin-triggered (D11). This one is not: once the one
//! record format is written everywhere, a database that still holds legacy
//! full-`Node` blobs should converge without anyone calling an endpoint. So
//! after the job system starts, the branches whose `node_path` state record on
//! THIS node is not `done` are backfilled — by ordinary `IndexRepair` jobs on
//! the unified job queue, streaming, bounded batches, resumable cursor, disk
//! precheck and rate limit exactly as the admin-triggered run (`run_repair`
//! with the default `RepairOptions`). Nothing runs on the boot path itself:
//! the first enqueue is spawned and waits [`AUTO_BACKFILL_DELAY`].
//!
//! **One branch at a time, as a chain.** On a first boot every branch is
//! pending, and one job per branch queued at once filled the background pool
//! with full-branch NODES scans — the write throttle is per run, the reads are
//! not throttled at all — so live writes' fulltext and embedding jobs waited
//! behind them. Instead ONE job is queued, marked [`AUTO_CHAIN_META`] in its
//! context; when it finishes (success or failure) the job handler calls
//! [`continue_node_path_backfill_chain`], which queues the next pending branch
//! AFTER it in `(tenant, repo, branch)` order. Walking strictly forward means
//! a branch that fails (no disk headroom, say) is not retried in a loop; it is
//! retried at the next start, as the backfill is cleanup, never the fix — the
//! read rule reads legacy blobs forever.
//!
//! `RAISIN_NODE_PATH_AUTO_BACKFILL=0` (or `false`/`off`/`no`) turns the
//! automatic chain off; the admin endpoint still works.
//!
//! The chain is generic over the repair kind: the localized name index's
//! initial build per branch (plan Phase 12, on by default) runs through the
//! same machinery as `localized_names` — see `crate::localized_name::auto` —
//! and so does the `property_index` rebuild that unlocks skip-unchanged
//! writes (plan Phase 7b) — see `auto_property_index`.

use super::branches::list_branches;
use super::enqueue::{enqueue_index_repair_with, list_repositories};
use super::{load_state, repair_node_id, RepairKind};
use crate::RocksDBStorage;
use raisin_error::Result;
use raisin_storage::jobs::{JobContext, JobStatus, JobType};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// The environment switch for the automatic backfill (default ON).
pub const NODE_PATH_AUTO_BACKFILL_ENV: &str = "RAISIN_NODE_PATH_AUTO_BACKFILL";

/// How long after the job system starts the first enqueue waits, so boot-time
/// work (restored jobs, replication catch-up) is not competing with a scan.
pub const AUTO_BACKFILL_DELAY: Duration = Duration::from_secs(30);

/// Job-context metadata key marking a job as a link of the automatic chain.
pub const AUTO_CHAIN_META: &str = "node_path_auto_chain";

/// `(tenant, repo, branch)`.
type BranchKey = (String, String, String);

/// Whether the automatic backfill is enabled (anything but `0`/`false`/`off`/
/// `no`, any case, or unset).
pub fn auto_backfill_enabled() -> bool {
    std::env::var(NODE_PATH_AUTO_BACKFILL_ENV)
        .map(|v| {
            !matches!(
                v.to_ascii_lowercase().as_str(),
                "0" | "false" | "off" | "no"
            )
        })
        .unwrap_or(true)
}

/// Every `(tenant, repo, branch)` whose `node_path` backfill has not reached
/// `done` on this node (no state record, or `running` after a crash), in
/// `(tenant, repo, branch)` order.
pub fn pending_node_path_branches(storage: &RocksDBStorage) -> Result<Vec<BranchKey>> {
    let db = storage.db();
    let node_id = repair_node_id(storage);
    let slug = RepairKind::NodePath.slug();
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

/// The branches an automatic chain of `kind` still owes, in order.
pub(super) fn pending_branches(
    storage: &RocksDBStorage,
    kind: RepairKind,
) -> Result<Vec<BranchKey>> {
    match kind {
        RepairKind::NodePath => pending_node_path_branches(storage),
        RepairKind::LocalizedNames => crate::localized_name::auto::pending_branches(storage),
        RepairKind::PropertyIndex => {
            super::auto_property_index::pending_property_index_branches(storage)
        }
        RepairKind::BlockOverlayTombstones => super::auto_block_overlays::pending_branches(storage),
        RepairKind::CompoundBuilds => super::compound_detect::pending_branches(storage),
        RepairKind::TimestampBackfill => super::auto_timestamps::pending_branches(storage),
        _ => Ok(Vec::new()),
    }
}

/// Start the chain: queue ONE job for the first pending branch — unless a
/// link of the chain is already queued or running (a restart restores an
/// interrupted one, and it continues the chain itself). Returns how many jobs
/// it queued (0 or 1).
pub async fn enqueue_pending_node_path_backfills(storage: &RocksDBStorage) -> Result<usize> {
    start_chain(storage, RepairKind::NodePath).await
}

/// [`enqueue_pending_node_path_backfills`] for any automatic chain `kind`
/// (`node_path`, `localized_names`).
pub async fn start_chain(storage: &RocksDBStorage, kind: RepairKind) -> Result<usize> {
    if chain_active(storage, kind).await {
        return Ok(0);
    }
    enqueue_next(storage, kind, None).await
}

/// Continue the chain after `context`'s job finished, whatever its outcome:
/// queue the next pending branch after `(tenant, repo, branch)` (`None`: the
/// whole repository). A job not marked [`AUTO_CHAIN_META`] — an admin or
/// checkpoint-ingest run — continues nothing. Returns how many jobs it queued.
pub async fn continue_node_path_backfill_chain(
    storage: &RocksDBStorage,
    context: &JobContext,
    tenant_id: &str,
    repo_id: &str,
    branch: Option<&str>,
) -> Result<usize> {
    if !auto_backfill_enabled() {
        return Ok(0);
    }
    continue_chain(
        storage,
        RepairKind::NodePath,
        context,
        tenant_id,
        repo_id,
        branch,
    )
    .await
}

/// [`continue_node_path_backfill_chain`] for any automatic chain `kind`.
pub async fn continue_chain(
    storage: &RocksDBStorage,
    kind: RepairKind,
    context: &JobContext,
    tenant_id: &str,
    repo_id: &str,
    branch: Option<&str>,
) -> Result<usize> {
    if !context.metadata.contains_key(AUTO_CHAIN_META) {
        return Ok(0);
    }
    let cursor = (
        tenant_id.to_string(),
        repo_id.to_string(),
        branch.map(str::to_string),
    );
    enqueue_next(storage, kind, Some(cursor)).await
}

/// Spawn the start of the chain: wait [`AUTO_BACKFILL_DELAY`], then
/// [`enqueue_pending_node_path_backfills`]. Never blocks the caller; a failure
/// is logged and retried at the next start.
pub fn schedule_node_path_backfill(storage: Arc<RocksDBStorage>) {
    if !auto_backfill_enabled() {
        tracing::info!(
            env = NODE_PATH_AUTO_BACKFILL_ENV,
            "automatic node_path backfill is disabled"
        );
        return;
    }
    schedule_chain(storage, RepairKind::NodePath, AUTO_BACKFILL_DELAY);
}

/// Spawn [`start_chain`] of `kind` after `delay`.
pub fn schedule_chain(storage: Arc<RocksDBStorage>, kind: RepairKind, delay: Duration) {
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        if let Err(e) = start_chain(&storage, kind).await {
            tracing::warn!(error = %e, repair = kind.slug(), "could not start an automatic repair chain");
        }
    });
}

/// Queue the first pending branch strictly after `after` that has no live
/// job of `kind` of its own (an admin run there finishes it) and whose link
/// has not failed in this process (`auto_targets`).
async fn enqueue_next(
    storage: &RocksDBStorage,
    kind: RepairKind,
    after: Option<(String, String, Option<String>)>,
) -> Result<usize> {
    for (tenant_id, repo_id, branch) in pending_branches(storage, kind)? {
        let is_after = match &after {
            None => true,
            Some((t, r, Some(b))) => (&tenant_id, &repo_id, &branch) > (t, r, b),
            Some((t, r, None)) => (&tenant_id, &repo_id) > (t, r),
        };
        if !is_after
            || super::auto_targets::link_failed(kind, &tenant_id, &repo_id, &branch)
            || branch_job_active(storage, kind, &tenant_id, &repo_id, &branch).await
        {
            continue;
        }
        let metadata = HashMap::from([(AUTO_CHAIN_META.to_string(), serde_json::json!(true))]);
        enqueue_index_repair_with(
            storage,
            &tenant_id,
            &repo_id,
            Some(&branch),
            kind,
            false,
            metadata,
            // No retries: the chain continues when a link FAILS too, and a
            // retry running beside the next link would fork the chain.
            Some(0),
        )
        .await?;
        tracing::info!(
            tenant_id,
            repo_id,
            branch,
            repair = kind.slug(),
            "automatic repair queued (one branch at a time)"
        );
        return Ok(1);
    }
    Ok(0)
}

/// A live (non-dry-run) job of `kind`.
fn live_job(
    kind: RepairKind,
    status: &JobStatus,
    job_type: &JobType,
) -> Option<(String, Option<String>)> {
    let live = matches!(
        status,
        JobStatus::Scheduled | JobStatus::Running | JobStatus::Executing
    );
    match job_type {
        JobType::IndexRepair {
            repo_id,
            branch,
            repair,
            dry_run: false,
            ..
        } if live && repair == kind.slug() => Some((repo_id.clone(), branch.clone())),
        _ => None,
    }
}

/// A live job of `kind` for exactly this branch, or for the whole
/// repository, is already in this process's registry.
pub(super) async fn branch_job_active(
    storage: &RocksDBStorage,
    kind: RepairKind,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> bool {
    storage
        .job_registry()
        .list_jobs_by_tenant(tenant_id)
        .await
        .into_iter()
        .filter_map(|job| live_job(kind, &job.status, &job.job_type))
        .any(|(r, b)| r == repo_id && b.as_deref().is_none_or(|b| b == branch))
}

/// A live link of the automatic chain of `kind` exists anywhere in this
/// process.
async fn chain_active(storage: &RocksDBStorage, kind: RepairKind) -> bool {
    storage
        .job_registry()
        .list_jobs()
        .await
        .into_iter()
        .filter(|job| live_job(kind, &job.status, &job.job_type).is_some())
        .any(|job| {
            storage
                .job_data_store()
                .get(&job.tenant, &job.id)
                .ok()
                .flatten()
                .is_some_and(|context| context.metadata.contains_key(AUTO_CHAIN_META))
        })
}
