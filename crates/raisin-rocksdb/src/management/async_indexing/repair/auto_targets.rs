//! Targeted links of the automatic repair chains (`auto_node_path`): a build
//! asked for ONE branch, and the branches whose link failed in this process.
//!
//! A chain walks strictly forward, so a failed link is not retried in a loop
//! by the chain itself — but a restart of the chain (`start_chain`, which a
//! localized lookup requests whenever it finds its branch not built) begins
//! at the first pending branch again. Without the failure record a branch
//! sorted first that keeps failing (no disk headroom, say) would be re-queued
//! on every request, and the branches after it never built. It is
//! process-local: the next start tries everything again.

use super::auto_node_path::{
    auto_backfill_enabled, branch_job_active, continue_chain, pending_branches,
};
use super::enqueue::enqueue_index_repair_with;
use super::RepairKind;
use crate::RocksDBStorage;
use raisin_error::Result;
use raisin_storage::jobs::JobContext;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

type Failed = HashSet<(&'static str, String, String, String)>;

fn failed() -> &'static Mutex<Failed> {
    static FAILED: OnceLock<Mutex<Failed>> = OnceLock::new();
    FAILED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Record how a link of `kind` on `(tenant, repo, branch)` ended (`branch`
/// `None`: the whole repository, which records nothing).
pub fn record_link_outcome(
    kind: RepairKind,
    tenant_id: &str,
    repo_id: &str,
    branch: Option<&str>,
    succeeded: bool,
) {
    let Some(branch) = branch else { return };
    let key = (
        kind.slug(),
        tenant_id.to_string(),
        repo_id.to_string(),
        branch.to_string(),
    );
    if let Ok(mut set) = failed().lock() {
        if succeeded {
            set.remove(&key);
        } else {
            set.insert(key);
        }
    }
}

/// Whether a link of `kind` on this branch failed in this process.
pub(super) fn link_failed(kind: RepairKind, tenant_id: &str, repo_id: &str, branch: &str) -> bool {
    failed().lock().is_ok_and(|set| {
        set.contains(&(
            kind.slug(),
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
        ))
    })
}

/// Queue a build of exactly `(tenant, repo, branch)` when it is pending and
/// has no live job — a standalone job, NOT a link of the chain (it continues
/// nothing, so it can never fork one). Returns how many jobs it queued.
pub async fn enqueue_branch(
    storage: &RocksDBStorage,
    kind: RepairKind,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<usize> {
    let wanted = (
        tenant_id.to_string(),
        repo_id.to_string(),
        branch.to_string(),
    );
    if !pending_branches(storage, kind)?.contains(&wanted)
        || branch_job_active(storage, kind, tenant_id, repo_id, branch).await
    {
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

/// A repair job of `kind` on `(tenant, repo, branch)` ended. For the
/// automatic chains (`node_path`, `localized_names`, `property_index`) record
/// the outcome and,
/// succeeded or not, queue the next pending branch (a link runs with no
/// retries, so this is the only continuation; a job not marked as a link —
/// an admin run — continues nothing).
pub async fn after_link(
    storage: &RocksDBStorage,
    kind: RepairKind,
    context: &JobContext,
    (tenant_id, repo_id, branch): (&str, &str, Option<&str>),
    succeeded: bool,
) {
    let continues = match kind {
        RepairKind::NodePath => auto_backfill_enabled(),
        RepairKind::LocalizedNames => crate::localized_name::enabled(),
        RepairKind::PropertyIndex => super::auto_property_index::auto_rebuild_enabled(storage),
        RepairKind::BlockOverlayTombstones => super::auto_block_overlays::auto_enabled(),
        RepairKind::CompoundBuilds => true,
        _ => return,
    };
    record_link_outcome(kind, tenant_id, repo_id, branch, succeeded);
    if !continues {
        return;
    }
    if let Err(e) = continue_chain(storage, kind, context, tenant_id, repo_id, branch).await {
        tracing::warn!(
            error = %e,
            repair = kind.slug(),
            "could not continue an automatic repair chain"
        );
    }
}
