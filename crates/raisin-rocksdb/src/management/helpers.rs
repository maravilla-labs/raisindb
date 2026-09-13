//! Helper functions for management operations.
//!
//! Provides tenant, repository, branch, and workspace enumeration
//! by scanning RocksDB column families.

use crate::{cf_handle, RocksDBStorage};
use raisin_error::Result;

/// List all repositories for a tenant
pub(super) async fn list_repositories_for_tenant(
    storage: &RocksDBStorage,
    tenant_id: &str,
) -> Result<Vec<String>> {
    let cf_registry = cf_handle(storage.db(), crate::cf::REGISTRY)?;
    let prefix = crate::keys::KeyBuilder::new()
        .push(tenant_id)
        .push("repos")
        .build_prefix();

    let mut repos = Vec::new();
    let iter = crate::prefix_scan(storage.db(), cf_registry, &prefix);

    for item in iter {
        let (key, _) =
            item.map_err(|e| raisin_error::Error::storage(format!("Iterator error: {}", e)))?;

        // `prefix_iterator_cf` is bounded by the CF's prefix EXTRACTOR, not by
        // the seek key: it positions at `prefix` and then keeps walking, so
        // every REGISTRY key sorting after `{tenant}\0repos\0` arrives here too.
        // Without this guard the `parts.len() >= 3` shape test below accepts
        // them and reports `parts[2]` as a repository — so a later tenant's
        // rows, or any other `{tenant}\0…\0…` namespace, showed up as phantom
        // (and duplicate) repositories. `list_repositories_for_tenant` on
        // `RepositoryManagementRepository` has always had this guard; these
        // hand-rolled copies did not.
        if !key.starts_with(&prefix) {
            break;
        }

        let key_str = String::from_utf8_lossy(&key);
        let parts: Vec<&str> = key_str.split('\0').collect();
        // EXACTLY three: `{tenant}\0repos\0{repo}` is the whole key shape (see
        // `keys::repository_key`). `>=` also matched anything nested under a
        // repository.
        if parts.len() == 3 {
            repos.push(parts[2].to_string());
        }
    }

    Ok(repos)
}

/// List all branches for a repository
pub(super) async fn list_branches_for_repo(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
) -> Result<Vec<String>> {
    let cf_branches = cf_handle(storage.db(), crate::cf::BRANCHES)?;
    let prefix = crate::keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("branches")
        .build_prefix();

    let mut branches = Vec::new();
    let iter = crate::prefix_scan(storage.db(), cf_branches, &prefix);

    for item in iter {
        let (key, _) =
            item.map_err(|e| raisin_error::Error::storage(format!("Iterator error: {}", e)))?;

        let key_str = String::from_utf8_lossy(&key);
        let parts: Vec<&str> = key_str.split('\0').collect();
        if parts.len() >= 4 {
            branches.push(parts[3].to_string());
        }
    }

    // Default to "main" if no branches found
    if branches.is_empty() {
        branches.push("main".to_string());
    }

    Ok(branches)
}

/// List all workspaces for a repository
pub(super) async fn list_workspaces_for_repo(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
) -> Result<Vec<String>> {
    let cf_workspaces = cf_handle(storage.db(), crate::cf::WORKSPACES)?;
    let prefix = crate::keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("workspaces")
        .build_prefix();

    let mut workspaces = Vec::new();
    let iter = crate::prefix_scan(storage.db(), cf_workspaces, &prefix);

    for item in iter {
        let (key, _) =
            item.map_err(|e| raisin_error::Error::storage(format!("Iterator error: {}", e)))?;

        let key_str = String::from_utf8_lossy(&key);
        let parts: Vec<&str> = key_str.split('\0').collect();
        if parts.len() >= 4 {
            workspaces.push(parts[3].to_string());
        }
    }

    // Default to "default" if no workspaces found
    if workspaces.is_empty() {
        workspaces.push("default".to_string());
    }

    Ok(workspaces)
}

/// List all tenants in the database
pub(super) async fn list_all_tenants(storage: &RocksDBStorage) -> Result<Vec<String>> {
    let cf_registry = cf_handle(storage.db(), crate::cf::REGISTRY)?;
    let prefix = crate::keys::KeyBuilder::new()
        .push("tenants")
        .build_prefix();

    let mut tenants = Vec::new();
    let iter = crate::prefix_scan(storage.db(), cf_registry, &prefix);

    for item in iter {
        let (key, _) =
            item.map_err(|e| raisin_error::Error::storage(format!("Iterator error: {}", e)))?;

        // Same over-read as `list_repositories_for_tenant` above: the iterator
        // walks past `tenants\0`, so any tenant whose id sorts after it (`zeta`
        // > `tenants`) contributed its own `{tenant}\0repos\0…` rows here and
        // was reported as a tenant literally named "repos".
        if !key.starts_with(&prefix) {
            break;
        }

        let key_str = String::from_utf8_lossy(&key);
        let parts: Vec<&str> = key_str.split('\0').collect();
        // `tenants\0{tenant}` is the whole shape (`keys::tenant_key`).
        if parts.len() == 2 {
            tenants.push(parts[1].to_string());
        }
    }

    Ok(tenants)
}

/// List all tenants in the database (public helper for background jobs)
pub async fn list_tenants(storage: &RocksDBStorage) -> Result<Vec<String>> {
    list_all_tenants(storage).await
}

/// List all repositories for a tenant (public helper for background jobs)
pub async fn list_repositories(storage: &RocksDBStorage, tenant_id: &str) -> Result<Vec<String>> {
    list_repositories_for_tenant(storage, tenant_id).await
}

/// Queue a build for every declared compound index that is not `Ready`,
/// across every tenant, repository, branch and workspace.
///
/// The steady-state and schema-change sweeps
/// (`jobs/event_handler/delete_and_schema_handlers.rs`) only fire off a
/// NodeType create/update or a workspace create — so a compound index whose
/// declaring schema event predates the binary that first tracked build state
/// (an upgrade, or state lost some other way — `cf::INDEX_STATUS` is excluded
/// from branch copy) has NO event left to ever re-fire, and the fail-closed
/// planner gate answers `NotBuilt` forever. This is the boot-time equivalent:
/// call it once at server startup so an index in that state self-heals
/// without requiring an operator to touch the NodeType by hand.
///
/// **Cheap in steady state, deliberately.** Enumeration is tenant/repo/branch/
/// workspace listing plus one NodeType listing per branch and one build-state
/// key lookup per declared index — no data scan. `sweep_compound_index_builds`
/// (see its doc) queues NOTHING for an index that already reads `Ready`, so a
/// normal boot on a healthy database does no rebuild work at all; the only
/// cost that scales with tenant count is the enumeration itself. Errors on one
/// tenant/repo/branch are logged and skipped — a single broken scope must not
/// block every other tenant's indexes from healing.
pub async fn sweep_compound_index_builds_at_boot(storage: &RocksDBStorage) -> Result<usize> {
    use raisin_storage::{BranchRepository, RepoScope, Storage, WorkspaceRepository};

    let mut queued = 0usize;

    let tenants = list_all_tenants(storage).await?;
    for tenant_id in tenants {
        let repos = match list_repositories_for_tenant(storage, &tenant_id).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(tenant = %tenant_id, error = %e, "boot compound-index sweep: failed to list repositories");
                continue;
            }
        };
        for repo_id in repos {
            let branches = match storage.branches().list_branches(&tenant_id, &repo_id).await {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!(tenant = %tenant_id, repo = %repo_id, error = %e, "boot compound-index sweep: failed to list branches");
                    continue;
                }
            };
            for branch in branches {
                let workspaces = match storage
                    .workspaces()
                    .list(RepoScope::new(&tenant_id, &repo_id))
                    .await
                {
                    Ok(w) => w,
                    Err(e) => {
                        tracing::warn!(tenant = %tenant_id, repo = %repo_id, branch = %branch.name, error = %e, "boot compound-index sweep: failed to list workspaces");
                        continue;
                    }
                };
                for workspace in workspaces {
                    match storage
                        .sweep_compound_index_builds(
                            &tenant_id,
                            &repo_id,
                            &branch.name,
                            &workspace.name,
                        )
                        .await
                    {
                        Ok(n) => queued += n,
                        Err(e) => {
                            tracing::warn!(
                                tenant = %tenant_id,
                                repo = %repo_id,
                                branch = %branch.name,
                                workspace = %workspace.name,
                                error = %e,
                                "boot compound-index sweep: failed to sweep workspace"
                            );
                        }
                    }
                }
            }
        }
    }

    if queued > 0 {
        tracing::info!(
            queued,
            "boot compound-index sweep: queued builds for indexes with no usable state"
        );
    }

    Ok(queued)
}

/// List only the repositories of `tenant_id` that hold a `raisin:VirtualMount`.
///
/// The fast path for the 60-second virtual-mount sync tick. [`list_repositories`]
/// answers "every repo", which the tick then fans a
/// `list_by_type("raisin:VirtualMount")` out to — 51% of production server CPU,
/// almost all of it spent on repositories that have never had a mount.
///
/// A `prefix_iterator_cf` over the `{tenant}\0vmounts\0…` namespace of the same
/// `REGISTRY` column family [`list_repositories_for_tenant`] scans, reading ids
/// out of KEYS without deserializing values. See [`crate::vmount_registry`] for
/// where the entries are written and for the reconcile that repairs drift.
pub async fn list_repos_with_virtual_mounts(
    storage: &RocksDBStorage,
    tenant_id: &str,
) -> Result<Vec<String>> {
    crate::vmount_registry::list_repos_with_mounts(storage.db(), tenant_id)
}

/// List all workspaces for a repository (public helper for background jobs)
pub async fn list_workspaces(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
) -> Result<Vec<String>> {
    list_workspaces_for_repo(storage, tenant_id, repo_id).await
}

/// List all branches for a repository (public helper for background jobs)
pub async fn list_branches(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
) -> Result<Vec<String>> {
    list_branches_for_repo(storage, tenant_id, repo_id).await
}
