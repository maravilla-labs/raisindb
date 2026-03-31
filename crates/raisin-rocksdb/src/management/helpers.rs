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
    let iter = storage.db().prefix_iterator_cf(cf_registry, &prefix);

    for item in iter {
        let (key, _) =
            item.map_err(|e| raisin_error::Error::storage(format!("Iterator error: {}", e)))?;

        let key_str = String::from_utf8_lossy(&key);
        let parts: Vec<&str> = key_str.split('\0').collect();
        if parts.len() >= 3 {
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
    let iter = storage.db().prefix_iterator_cf(cf_branches, &prefix);

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
    let iter = storage.db().prefix_iterator_cf(cf_workspaces, &prefix);

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
    let iter = storage.db().prefix_iterator_cf(cf_registry, &prefix);

    for item in iter {
        let (key, _) =
            item.map_err(|e| raisin_error::Error::storage(format!("Iterator error: {}", e)))?;

        let key_str = String::from_utf8_lossy(&key);
        let parts: Vec<&str> = key_str.split('\0').collect();
        if parts.len() >= 2 {
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
