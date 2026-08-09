//! Backfill and self-heal for the virtual-mount registry.
//!
//! [`crate::vmount_registry`] makes the 60-second sync tick O(repos with mounts)
//! instead of O(all tenants × all repos). That is only safe if the registry is
//! complete, and it can be incomplete for two reasons:
//!
//! 1. **History.** Every mount created before the registry existed has no entry.
//! 2. **Drift.** A future write path that stages a mount node without staging
//!    its entry would strand that mount. The failure is SILENT — the mount just
//!    never syncs, no error is raised anywhere — which is the same failure class
//!    CLAUDE.md's "one walker" invariants exist to prevent for the reference and
//!    spatial indexes.
//!
//! Both are answered by the same code, deliberately: a separate one-shot
//! migration and a separate reconcile would be two implementations of "find the
//! mounts", free to disagree. [`reconcile_registry`] is the full
//! `list_tenants` → `list_repositories` → `list_by_type` enumeration — i.e.
//! exactly the scan the fast path replaced — used now only as an audit.
//!
//! It runs on the FIRST sync tick of a process (that is the migration) and
//! hourly thereafter (that is the self-heal), driven from
//! [`super::check::run_check`]. Hourly, one full enumeration is ~1/60th of what
//! this change removed, so the audit does not reintroduce the cost.
//!
//! A repair is logged at **warn** with the mount named, because after the first
//! pass a repair means a write path is missing its registry write and someone
//! has to go fix it. A prune is logged at debug: entries for branches no scan
//! reads (a fork's copy of a mount config node) are expected and harmless.

use std::sync::Arc;

use raisin_core::services::node_service::NodeService;
use raisin_error::Result;
use raisin_models::auth::AuthContext;
use rocksdb::WriteBatch;

use super::check::config_branch;
use super::config::SYSTEM_WORKSPACE;
use crate::vmount_registry::{self, MOUNT_NODE_TYPE};
use crate::RocksDBStorage;

/// What one reconcile pass found.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ReconcileReport {
    /// `(tenant, repo)` pairs enumerated.
    pub repos_scanned: usize,
    /// Mount nodes found by the authoritative scan.
    pub mounts_found: usize,
    /// Entries the fast path was MISSING and this pass wrote. Non-zero after the
    /// first pass means a write path has drifted.
    pub repaired: usize,
    /// Entries pointing at mounts that no longer exist (or that live on a branch
    /// no scan reads), removed.
    pub pruned: usize,
}

/// Rebuild the registry from the authoritative node data.
///
/// Repairs missing entries and prunes dead ones. `tenant_filter` / `repo_filter`
/// narrow the walk the same way [`super::check::run_check`]'s do.
pub(crate) async fn reconcile_registry(
    storage: &Arc<RocksDBStorage>,
    tenant_filter: Option<String>,
    repo_filter: Option<String>,
) -> Result<ReconcileReport> {
    let mut report = ReconcileReport::default();

    let tenants = match &tenant_filter {
        Some(t) => vec![t.clone()],
        None => crate::management::list_tenants(storage).await?,
    };

    for tenant in tenants {
        let repos = match &repo_filter {
            Some(r) => vec![r.clone()],
            None => match crate::management::list_repositories(storage, &tenant).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(tenant = %tenant, error = %e, "vmount-registry: list repos failed");
                    continue;
                }
            },
        };

        // Everything the fast path currently believes about this tenant. Only
        // pruned against repos this pass actually managed to scan — a repo whose
        // scan failed must keep its entries, or one transient error would delete
        // the very index this reconcile exists to protect.
        let mut stale: Vec<vmount_registry::RegistryEntry> =
            match vmount_registry::list_entries(storage.db(), &tenant) {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(tenant = %tenant, error = %e, "vmount-registry: read failed");
                    Vec::new()
                }
            };
        let mut scanned_repos: Vec<String> = Vec::new();

        for repo in repos {
            report.repos_scanned += 1;
            let branch = config_branch(storage, &tenant, &repo).await;

            let svc: NodeService<RocksDBStorage> = NodeService::new_with_context(
                storage.clone(),
                tenant.clone(),
                repo.clone(),
                branch.clone(),
                SYSTEM_WORKSPACE.to_string(),
            )
            .with_auth(AuthContext::system());

            let mounts = match svc.list_by_type(MOUNT_NODE_TYPE).await {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(
                        tenant = %tenant, repo = %repo, error = %e,
                        "vmount-registry: mount scan failed; entries for this repo left alone"
                    );
                    continue;
                }
            };
            scanned_repos.push(repo.clone());
            report.mounts_found += mounts.len();

            for node in &mounts {
                // This mount is accounted for; whatever else is registered for
                // this repo is a candidate for pruning.
                stale.retain(|e| !(e.repo == repo && e.branch == branch && e.mount_id == node.id));

                match vmount_registry::contains(storage.db(), &tenant, &repo, &branch, &node.id) {
                    Ok(true) => {}
                    Ok(false) => {
                        let mut batch = WriteBatch::default();
                        vmount_registry::record_node_write(
                            &mut batch,
                            storage.db(),
                            &tenant,
                            &repo,
                            &branch,
                            node,
                        )?;
                        storage.db().write(batch).map_err(|e| {
                            raisin_error::Error::storage(format!("Registry repair failed: {}", e))
                        })?;
                        report.repaired += 1;
                        tracing::warn!(
                            tenant = %tenant, repo = %repo, branch = %branch, mount_id = %node.id,
                            "vmount-registry: MISSING entry repaired — a mount was invisible to \
                             the fast sync scan. If this is not the first pass after startup, a \
                             node write path is failing to stage its registry entry."
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            tenant = %tenant, repo = %repo, mount_id = %node.id, error = %e,
                            "vmount-registry: entry probe failed"
                        );
                    }
                }
            }
        }

        for entry in stale {
            if !scanned_repos.contains(&entry.repo) {
                continue;
            }
            let cf_registry = match crate::cf_handle(storage.db(), crate::cf::REGISTRY) {
                Ok(h) => h,
                Err(e) => {
                    tracing::warn!(error = %e, "vmount-registry: prune failed");
                    break;
                }
            };
            let key =
                vmount_registry::registry_key(&tenant, &entry.repo, &entry.branch, &entry.mount_id);
            if let Err(e) = storage.db().delete_cf(cf_registry, key) {
                tracing::warn!(error = %e, "vmount-registry: prune delete failed");
                continue;
            }
            report.pruned += 1;
            tracing::debug!(
                tenant = %tenant, repo = %entry.repo, branch = %entry.branch,
                mount_id = %entry.mount_id,
                "vmount-registry: pruned an entry with no mount behind it"
            );
        }
    }

    Ok(report)
}
