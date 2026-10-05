//! Automatic, background builds of the localized name index (owner decision,
//! 2026-10-04: on by default for every repository).
//!
//! The build is the `localized_names` repair, queued as ordinary
//! `IndexRepair` jobs on the unified job queue, one branch at a time, through
//! the same chain as the `node_path` backfill (`repair::auto_node_path`) —
//! never on the boot path itself:
//!
//! - after the job system starts ([`schedule_after_start`]), for every branch
//!   with a workspace that is not `Ready` under its repository's current
//!   fingerprint;
//! - whenever a lookup finds its branch not built ([`request_build`]) — a
//!   fork, a publish target, a configuration change (local or replicated), a
//!   checkpoint ingest, a merge from an unbuilt source — and right after a
//!   configuration change, local or replicated. The REQUESTED branch is
//!   queued directly (a standalone job) when it has no live job, and the
//!   chain is started for everything else pending (a no-op while it runs).
//!
//! Every link of the chain queues the next pending branch when it ends
//! (`jobs::handlers::maintenance`), so one start builds every branch; a
//! branch whose link failed is skipped until the next start
//! (`repair::auto_targets`).
//!
//! With the index switched off ([`super::enabled`]) nothing is queued, and at
//! start every state record is reset to `NotBuilt`: writes made while it is
//! off are not indexed, so switching it back on must rebuild.

use super::{config, state};
use crate::management::async_indexing::repair::{
    debounced, enqueue_branch, list_branches, list_repositories, register_requester,
    registered_storages, schedule_chain, start_chain, RepairKind,
};
use crate::RocksDBStorage;
use raisin_error::Result;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// How long after the job system starts the first build is queued.
pub const START_DELAY: Duration = Duration::from_secs(45);

type BranchKey = (String, String, String);

/// Every `(tenant, repo, branch)` with a workspace whose record is not
/// `Ready` under the repository's current fingerprint, in order.
pub fn pending_branches(storage: &RocksDBStorage) -> Result<Vec<BranchKey>> {
    if !super::enabled() {
        return Ok(Vec::new());
    }
    let db = storage.db();
    let mut out = Vec::new();
    for (tenant_id, repo_id) in list_repositories(db)? {
        let Some(cfg) = config::load(db, &tenant_id, &repo_id)? else {
            continue;
        };
        let fingerprint = cfg.fingerprint();
        let workspaces = super::rebuild::list_workspaces(db, &tenant_id, &repo_id)?;
        for branch in list_branches(db, &tenant_id, &repo_id)? {
            let mut pending = false;
            for workspace in &workspaces {
                let record = state::read(db, &tenant_id, &repo_id, &branch, workspace)?;
                if !state::availability(record.as_ref(), &fingerprint, None).is_ready() {
                    pending = true;
                    break;
                }
            }
            if pending {
                out.push((tenant_id.clone(), repo_id.clone(), branch));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Called once the job system runs: register `storage` for build requests
/// and queue the first builds after [`START_DELAY`]. With the index off,
/// reset every state record instead.
pub fn schedule_after_start(storage: Arc<RocksDBStorage>) {
    if !super::enabled() {
        match state::mark_all_not_built(storage.db()) {
            Ok(n) => tracing::info!(
                records = n,
                env = super::LOCALIZED_NAME_INDEX_ENV,
                "localized name index is switched off; state records reset to NotBuilt"
            ),
            Err(e) => tracing::warn!(error = %e, "could not reset localized name state"),
        }
        return;
    }
    register_requester(&storage);
    schedule_chain(storage, RepairKind::LocalizedNames, START_DELAY);
}

/// Ask for the build of `(tenant, repo, branch)` (`branch` empty: the whole
/// repository) and of anything else pending, in the background. Never blocks
/// and never fails the caller; without a running job system (no registered
/// storage) it does nothing.
pub fn request_build(tenant_id: &str, repo_id: &str, branch: &str) {
    let target = (
        tenant_id.to_string(),
        repo_id.to_string(),
        branch.to_string(),
    );
    if !super::enabled() || debounced(RepairKind::LocalizedNames, &target) {
        return;
    }
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let storages: Vec<Arc<RocksDBStorage>> = registered_storages();
    for storage in storages {
        let target = target.clone();
        handle.spawn(async move {
            let kind = RepairKind::LocalizedNames;
            let (t, r, b) = (&target.0, &target.1, &target.2);
            let direct = if b.is_empty() {
                Ok(0)
            } else {
                enqueue_branch(&storage, kind, t, r, b).await
            };
            let chained = start_chain(&storage, kind).await;
            match (direct, chained) {
                (Ok(0), Ok(0)) => {}
                (Ok(_), Ok(_)) => {
                    tracing::debug!(?target, "localized name build queued on request")
                }
                (Err(e), _) | (_, Err(e)) => {
                    tracing::warn!(error = %e, "could not queue a localized name build")
                }
            }
        });
    }
}
