//! The periodic driver for the reconcile walk.
//!
//! Mirrors [`super::super::check::run_check`]: enumerate tenants → repos → the
//! `raisin:system` workspace → `raisin:VirtualMount` nodes, and do one bounded
//! unit of work per mount. It differs in one way that matters — the sync check
//! only ENQUEUES, whereas this walks and WRITES — which is what forces the
//! lease below.

use std::sync::Arc;

use raisin_core::services::node_service::NodeService;
use raisin_error::Result;
use raisin_locks::LockManagerHandle;
use raisin_models::auth::AuthContext;

use super::super::check;
use super::super::config::{MountConfig, SYSTEM_WORKSPACE};
use super::super::ctx::{mount_lease_key, SYNC_LEASE_TTL};
use super::super::{persist_mount_state, read_mount_state};
use super::reconcile::{reconcile_mount, RECONCILE_REVISION_LIMIT};
use crate::RocksDBStorage;

/// Walk every writable mount's revisions from its watermark.
///
/// Returns the number of mounts reconciled (not the number of changes found).
pub(crate) async fn run_write_reconcile(
    storage: &Arc<RocksDBStorage>,
    lock_manager: Option<&LockManagerHandle>,
    instance_id: &str,
    tenant_filter: Option<String>,
    repo_filter: Option<String>,
) -> Result<usize> {
    let mut walked = 0usize;
    for (tenant, repo) in check::scan_scope(storage, tenant_filter, repo_filter).await? {
        match reconcile_repo(storage, lock_manager, instance_id, &tenant, &repo).await {
            Ok(n) => walked += n,
            Err(e) => {
                tracing::warn!(
                    tenant = %tenant, repo = %repo, error = %e,
                    "vmount-write-reconcile: repo scan failed"
                );
            }
        }
    }
    Ok(walked)
}

async fn reconcile_repo(
    storage: &Arc<RocksDBStorage>,
    lock_manager: Option<&LockManagerHandle>,
    instance_id: &str,
    tenant: &str,
    repo: &str,
) -> Result<usize> {
    let branch = check::config_branch(storage, tenant, repo).await;
    let svc: NodeService<RocksDBStorage> = NodeService::new_with_context(
        storage.clone(),
        tenant.to_string(),
        repo.to_string(),
        branch.clone(),
        SYSTEM_WORKSPACE.to_string(),
    )
    .with_auth(AuthContext::system());

    let mut walked = 0usize;
    for node in svc.list_by_type("raisin:VirtualMount").await? {
        let Ok(mount) = MountConfig::from_node(&node) else {
            // The sync check already records the misconfigured verdict on this
            // mount, on the same tick. Doing it again here would just fight it.
            continue;
        };
        if !should_reconcile(&mount) {
            continue;
        }
        // Per-mount, not per-repo: one mount with an unparseable state blob (or
        // a read that fails) must not stop every other mount in the repository
        // from reconciling. A walk that never runs is silent by construction —
        // its whole output is a watermark nobody looks at — so a shared failure
        // would go unnoticed for exactly as long as it took the pre-images to
        // age out.
        match reconcile_one(
            storage,
            lock_manager,
            instance_id,
            tenant,
            repo,
            &branch,
            &mount,
        )
        .await
        {
            Ok(true) => walked += 1,
            Ok(false) => {}
            Err(e) => tracing::warn!(
                mount_id = %mount.mount_id,
                error = %e,
                "vmount-write-reconcile: mount walk failed"
            ),
        }
    }
    Ok(walked)
}

/// Whether this mount needs a durable change feed.
///
/// A PAUSED mount is deliberately included. Pausing stops the scheduler; it does
/// not stop a user editing or deleting the nodes the mount owns, and a delete
/// only exists as long as its MVCC pre-image does. Skipping paused mounts would
/// make "pause, tidy up the mailbox, resume" silently unrepresentable — the
/// deletes would be gone before anything could see them. Detection writes only
/// this mount's own state and never calls a provider, so it is safe under a
/// pause in a way that a drain would not be.
fn should_reconcile(mount: &MountConfig) -> bool {
    // A disabled mount has had its push subscription torn down and is not
    // materializing anything; its watermark resumes wherever it left off if it
    // is ever re-enabled.
    if !mount.enabled {
        return false;
    }
    // Detection follows the mount's INTENT to write, not the engine's current
    // ability to honour it — see `WriteConfig::wants_any_write`.
    mount.write_config.wants_any_write()
}

/// Reconcile one mount under its lease. `Ok(false)` means another node holds it.
#[allow(clippy::too_many_arguments)]
async fn reconcile_one(
    storage: &Arc<RocksDBStorage>,
    lock_manager: Option<&LockManagerHandle>,
    instance_id: &str,
    tenant: &str,
    repo: &str,
    branch: &str,
    mount: &MountConfig,
) -> Result<bool> {
    // The lease is NOT optional cluster hygiene here. Job dedup is an in-memory
    // `HashMap` in `JobRegistry` and therefore per-PROCESS, so on an N-node
    // cluster every node runs its own copy of this periodic job. Two of them
    // walking the same mount would both advance the watermark and both write
    // `state`, and the `state_seq` CAS means the loser silently skips its write
    // — a watermark advanced by one node while the other's recorded deletes are
    // thrown away.
    let lock_key = mount_lease_key(tenant, repo, branch, &mount.mount_id);
    let lease = match lock_manager {
        Some(lm) => {
            let owner = format!("{instance_id}:reconcile:{}", mount.mount_id);
            match lm.try_acquire(&lock_key, &owner, SYNC_LEASE_TTL).await? {
                Some(guard) => Some(guard.token),
                None => {
                    // A sync run or another node's walk holds it. Doing nothing
                    // is correct: the watermark did not move, so the next tick
                    // resumes from exactly the same place.
                    tracing::debug!(
                        mount_id = %mount.mount_id,
                        "mount is leased elsewhere; skipping this reconcile pass"
                    );
                    return Ok(false);
                }
            }
        }
        None => None,
    };

    let outcome = reconcile_locked(storage, tenant, repo, branch, mount).await;

    if let (Some(lm), Some(token)) = (lock_manager, lease) {
        // Best-effort: the lease expires on its own, and failing the pass
        // because a release failed would be worse than the leak.
        let _ = lm.release(&lock_key, token).await;
    }
    outcome.map(|_| true)
}

async fn reconcile_locked(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    branch: &str,
    mount: &MountConfig,
) -> Result<()> {
    // Re-read the state under the lease rather than trusting the copy from the
    // listing above: the listing is a snapshot from before the lease was taken,
    // and persisting from it would roll back whatever landed in between (its
    // `state_seq` would also be stale, so the write would simply be refused —
    // silently, which is worse).
    let Some(mut state) = read_mount_state(storage, tenant, repo, branch, &mount.mount_id).await?
    else {
        return Ok(());
    };

    let findings = reconcile_mount(
        storage,
        tenant,
        repo,
        mount,
        &mut state,
        RECONCILE_REVISION_LIMIT,
    )
    .await?;

    if findings.deletes_found > 0 || findings.local_changes > 0 || findings.initialized {
        tracing::info!(
            mount_id = %mount.mount_id,
            initialized = findings.initialized,
            scanned = findings.scanned,
            echoes = findings.echoes,
            local_changes = findings.local_changes,
            deletes_found = findings.deletes_found,
            truncated = findings.truncated,
            "virtual mount write reconcile"
        );
    }

    persist_mount_state(storage, tenant, repo, branch, &mount.mount_id, &mut state).await?;
    Ok(())
}
