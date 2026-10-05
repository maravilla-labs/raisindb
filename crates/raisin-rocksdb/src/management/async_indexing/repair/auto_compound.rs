//! Who queues the `compound_builds` repair (plan Phase 13f): the chain after
//! start, a targeted link on request, and the checkpoint-ingest restart.

use super::auto_node_path::{branch_job_active, schedule_chain, start_chain};
use super::compound_builds::START_DELAY;
use super::compound_detect::{branch_work, work_fingerprint};
use super::enqueue::enqueue_index_repair_with;
use super::requests::{debounced, registered_storage_for};
use super::{repair_node_id, RepairKind};
use crate::RocksDBStorage;
use raisin_error::Result;
use rocksdb::DB;
use std::collections::HashMap;
use std::sync::Arc;

/// Called once the job system runs: start the chain after [`START_DELAY`].
pub fn schedule_after_start(storage: Arc<RocksDBStorage>) {
    schedule_chain(storage, RepairKind::CompoundBuilds, START_DELAY);
}

/// Ask for the `compound_builds` link of `(tenant, repo, branch)` on the
/// storage over `db`, in the background: queued when the data shows owed
/// work there and no link of that branch is live. A standalone job (it
/// continues no chain). Never blocks and never fails the caller; without a
/// running job system over `db` it does nothing (the next start finds the
/// work from the data).
pub fn request(db: &DB, tenant_id: &str, repo_id: &str, branch: &str) {
    let Some(storage) = registered_storage_for(db) else {
        return;
    };
    let target = (
        tenant_id.to_string(),
        repo_id.to_string(),
        branch.to_string(),
    );
    if debounced(RepairKind::CompoundBuilds, &target) {
        return;
    }
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    handle.spawn(async move {
        if let Err(e) = enqueue_if_owed(&storage, &target.0, &target.1, &target.2).await {
            tracing::warn!(error = %e, "could not queue a compound_builds link");
        }
    });
}

/// Queue the targeted link when the branch owes work and has none live.
/// Returns how many jobs it queued (0 or 1).
///
/// A branch whose last link FAILED (or was refused for missing order-column
/// values, plan Phase 13g) is skipped while the work it owes is the
/// work it failed on (`work_fingerprint` on its state record): the plan's "no
/// retries, a failed branch retried at the next start". Without this every
/// workspace event and cold drain re-queued a full link that re-scanned the
/// workspace and failed the same way, indefinitely. A change in the owed
/// work (an index switched off, a workspace added) runs it again at once.
pub async fn enqueue_if_owed(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<usize> {
    enqueue_owed(storage, tenant_id, repo_id, branch, false).await
}

/// After the `timestamp_backfill` repair rewrote nodes of `(tenant, repo,
/// branch)` (plan Phase 13g): queue the branch's link even though its last
/// link was refused on that very work — the backfill is what changed, and
/// the owed-work fingerprint cannot see it. Queued even when the record owes
/// nothing: a user-declared workspace or NodeType index whose own
/// `CompoundIndexBuild` job refused records nothing a request could see,
/// and the link builds every unready index (`branch_work(all_unready)`). A
/// link that FAILED (no headroom, unplaceable nodes) on unchanged work is
/// still left to the next start. Returns how many jobs it queued (0 or 1).
///
/// A link of the branch that is live right now may have judged the nodes
/// before the backfill reached them; it would save the refusal and nothing
/// would re-run it before the next start. So while one is live, a background
/// task waits for it to end (polling, bounded by [`AFTER_BACKFILL_WAIT`]) and
/// asks again.
pub async fn request_after_backfill(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<usize> {
    let kind = RepairKind::CompoundBuilds;
    if !branch_job_active(storage, kind, tenant_id, repo_id, branch).await {
        return enqueue_owed(storage, tenant_id, repo_id, branch, true).await;
    }
    let (Some(storage), Ok(handle)) = (
        registered_storage_for(storage.db()),
        tokio::runtime::Handle::try_current(),
    ) else {
        return Ok(0);
    };
    let target = (
        tenant_id.to_string(),
        repo_id.to_string(),
        branch.to_string(),
    );
    handle.spawn(async move {
        let (t, r, b) = (&target.0, &target.1, &target.2);
        for _ in 0..AFTER_BACKFILL_POLLS {
            tokio::time::sleep(AFTER_BACKFILL_POLL).await;
            if branch_job_active(&storage, kind, t, r, b).await {
                continue;
            }
            if let Err(e) = enqueue_owed(&storage, t, r, b, true).await {
                tracing::warn!(error = %e, "could not queue a compound_builds link after a backfill");
            }
            return;
        }
        tracing::info!(
            tenant_id = %t,
            repo_id = %r,
            branch = %b,
            "a compound_builds link stayed live past the backfill's wait; the next start builds what it refused"
        );
    });
    Ok(0)
}

/// How often, and how many times, [`request_after_backfill`] looks for the
/// live link to end.
const AFTER_BACKFILL_POLL: std::time::Duration = std::time::Duration::from_secs(5);
const AFTER_BACKFILL_POLLS: u32 = 120;
/// The bound on that wait (10 minutes).
pub const AFTER_BACKFILL_WAIT: std::time::Duration =
    std::time::Duration::from_secs(5 * AFTER_BACKFILL_POLLS as u64);

/// `after_backfill`: queue even when nothing is owed, and past a refusal's
/// fingerprint (never past a failure's) — see [`request_after_backfill`].
async fn enqueue_owed(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    after_backfill: bool,
) -> Result<usize> {
    let kind = RepairKind::CompoundBuilds;
    let db = storage.db();
    let state = super::load_state(
        db,
        tenant_id,
        repo_id,
        branch,
        kind.slug(),
        &repair_node_id(storage),
    )?;
    let owed_work = branch_work(db, tenant_id, repo_id, branch, false)?;
    let owed = after_backfill
        || state.as_ref().is_none_or(|s| s.status != "done")
        || !owed_work.is_empty();
    if !owed {
        return Ok(0);
    }
    let ended_short = |s: &super::RepairState| {
        s.status == "failed"
            || (!after_backfill && s.status == super::compound_builds::REFUSED_STATUS)
    };
    if let Some(failed) = state.filter(ended_short) {
        if failed.epoch.as_deref() == Some(work_fingerprint(&owed_work).as_str()) {
            tracing::debug!(
                tenant_id,
                repo_id,
                branch,
                "compound_builds: the branch's last link failed on the same work; \
                 retried at the next start"
            );
            return Ok(0);
        }
    }
    if branch_job_active(storage, kind, tenant_id, repo_id, branch).await {
        return Ok(0);
    }
    enqueue_index_repair_with(
        storage,
        tenant_id,
        repo_id,
        Some(branch),
        kind,
        false,
        HashMap::new(),
        Some(0),
    )
    .await?;
    Ok(1)
}

/// After a checkpoint ingest marked every compound record `NotBuilt`: every
/// branch is owed a link again, and the chain restarts (a no-op while one
/// runs).
pub async fn restart_after_ingest(storage: &RocksDBStorage) -> Result<usize> {
    let node_id = repair_node_id(storage);
    for (tenant_id, repo_id) in super::list_repositories(storage.db())? {
        super::mark_repairs_pending(
            storage.db(),
            &tenant_id,
            &repo_id,
            &node_id,
            &[RepairKind::CompoundBuilds],
        )?;
    }
    start_chain(storage, RepairKind::CompoundBuilds).await
}
