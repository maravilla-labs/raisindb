//! The `timestamp_backfill` repair runs by itself (plan Phase 13g, owner
//! request of 2026-10-05).
//!
//! Production after 0.7.0: the `compound_builds` repair refused the built-in
//! `(__parent_path, __created_at)` index on workspaces holding nodes written
//! before the write layer stamped `created_at` / `updated_at` — nobody would
//! run a backfill by hand for that, so like the other upgrade repairs it
//! queues itself: after the job system starts, the branches whose
//! `timestamp_backfill` state record on THIS node is not `done` are backfilled
//! by ordinary `IndexRepair` jobs, one branch at a time, through the shared
//! chain (`auto_node_path`), never on the boot path. A checkpoint ingest marks
//! every branch pending again and restarts the chain (the peer's checkpoint
//! can carry legacy versions); a branch whose `compound_builds` link was
//! refused for missing order-column values is pending again too. A branch
//! that completes having written something re-requests its `compound_builds`
//! link (`timestamp_backfill.rs`).
//!
//! `RAISIN_TIMESTAMP_BACKFILL=0` (or `false`/`off`/`no`) turns the automatic
//! run off; the admin endpoint still works. With it off, the affected
//! workspaces' built-in listings keep scanning (correct, slower) and the
//! refusal stays recorded on the `compound_builds` state.

use super::auto_node_path::schedule_chain;
use super::branches::list_branches;
use super::enqueue::list_repositories;
use super::{load_state, repair_node_id, RepairKind};
use crate::RocksDBStorage;
use raisin_error::Result;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// The environment switch for the automatic backfill (default ON).
pub const TIMESTAMP_BACKFILL_ENV: &str = "RAISIN_TIMESTAMP_BACKFILL";

/// Before the `compound_builds` chain's first link (120 s), so on an upgrade
/// the first branches are usually backfilled before their builds are judged;
/// the re-request after each branch covers the rest.
pub const START_DELAY: Duration = Duration::from_secs(100);

/// `(tenant, repo, branch)`.
type BranchKey = (String, String, String);

/// A process-wide TEST override of [`TIMESTAMP_BACKFILL_ENV`] (`None`: read
/// the environment) — the environment is shared by parallel tests.
pub fn override_auto(enabled: Option<bool>) {
    let value = match enabled {
        None => 0,
        Some(true) => 1,
        Some(false) => 2,
    };
    OVERRIDE.store(value, Ordering::Relaxed);
}

static OVERRIDE: AtomicU8 = AtomicU8::new(0);

/// Whether the automatic backfill is enabled (anything but `0`/`false`/`off`/
/// `no`, any case, or unset).
pub fn auto_enabled() -> bool {
    match OVERRIDE.load(Ordering::Relaxed) {
        1 => return true,
        2 => return false,
        _ => {}
    }
    std::env::var(TIMESTAMP_BACKFILL_ENV)
        .map(|v| {
            !matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "off" | "no"
            )
        })
        .unwrap_or(true)
}

/// Every `(tenant, repo, branch)` whose backfill has not reached `done` on
/// this node, or whose `compound_builds` link was refused for missing
/// order-column values since (a version without timestamps written after the
/// backfill finished: a replicated legacy version, a backup import, the edit
/// paths that do not stamp), in order (empty while the automatic backfill is
/// off).
pub fn pending_branches(storage: &RocksDBStorage) -> Result<Vec<BranchKey>> {
    if !auto_enabled() {
        return Ok(Vec::new());
    }
    let db = storage.db();
    let node_id = repair_node_id(storage);
    let slug = RepairKind::TimestampBackfill.slug();
    let mut out = Vec::new();
    for (tenant_id, repo_id) in list_repositories(db)? {
        for branch in list_branches(db, &tenant_id, &repo_id)? {
            let state = load_state(db, &tenant_id, &repo_id, &branch, slug, &node_id)?;
            let refused = || -> Result<bool> {
                Ok(load_state(
                    db,
                    &tenant_id,
                    &repo_id,
                    &branch,
                    RepairKind::CompoundBuilds.slug(),
                    &node_id,
                )?
                .is_some_and(|s| s.status == super::compound_builds::REFUSED_STATUS))
            };
            if state.is_none_or(|s| s.status != "done") || refused()? {
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
            env = TIMESTAMP_BACKFILL_ENV,
            "automatic timestamp_backfill is off"
        );
        return;
    }
    schedule_chain(storage, RepairKind::TimestampBackfill, START_DELAY);
}

/// After a checkpoint ingest: every branch is owed the backfill again (the
/// peer's records may carry legacy versions), and the chain restarts (a no-op
/// while one runs). Nothing while the automatic backfill is off.
pub async fn restart_after_ingest(storage: &RocksDBStorage) -> Result<usize> {
    if !auto_enabled() {
        return Ok(0);
    }
    let node_id = repair_node_id(storage);
    for (tenant_id, repo_id) in list_repositories(storage.db())? {
        super::mark_repairs_pending(
            storage.db(),
            &tenant_id,
            &repo_id,
            &node_id,
            &[RepairKind::TimestampBackfill],
        )?;
    }
    super::auto_node_path::start_chain(storage, RepairKind::TimestampBackfill).await
}
