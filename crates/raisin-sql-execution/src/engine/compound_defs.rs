//! The compound-index declarations handed to the physical planner: the
//! NodeType-owned ones (`helpers::load_compound_indexes` /
//! `load_all_compound_indexes`) plus the WORKSPACE-owned ones of the workspace
//! the statement reads (plan Phase 13e).
//!
//! One entry point for every planner caller (SELECT, non-bulk and bulk DML),
//! so a workspace index cannot be visible to one door and not another.
//!
//! Workspace declarations come off the workspace record through
//! `Workspace::owned_compound_indexes` — the same derivation the storage
//! writers and builds use, so the planner and the keyspace agree on the
//! stored name (`@{name}`) and the owner. Cached in the same short-TTL cache
//! as the NodeType declarations: a stale entry is safe because availability
//! is judged against the build state (`CompoundAvailability`), which fails
//! closed on a declaration it was not built from.

use super::helpers;
use raisin_models::nodes::properties::schema::CompoundIndexDefinition;
use raisin_storage::{RepoScope, Storage, WorkspaceRepository};
use std::sync::Arc;

/// The workspace's own compound indexes (stored names, owner stamped).
pub(crate) async fn load_workspace_compound_indexes<S: Storage>(
    storage: &S,
    tenant_id: &str,
    repo_id: &str,
    workspace: &str,
) -> Vec<CompoundIndexDefinition> {
    let key = format!("workspace\0{tenant_id}\0{repo_id}\0{workspace}");
    let cache = helpers::compound_index_cache();
    if let Some(cached) = cache.get(&key) {
        return (*cached).clone();
    }
    let loaded = match storage
        .workspaces()
        .get(RepoScope::new(tenant_id, repo_id), workspace)
        .await
    {
        Ok(Some(ws)) => ws.owned_compound_indexes(),
        Ok(None) => Vec::new(),
        Err(e) => {
            // Not cached: an unreadable record plans without its indexes
            // (a scan — correct) and is retried on the next statement.
            tracing::warn!(workspace, error = %e, "could not read workspace compound indexes");
            return Vec::new();
        }
    };
    cache.put(&key, Arc::new(loaded.clone()));
    loaded
}

/// Everything the planner may choose from for a statement on `workspace`
/// (`None`: no workspace known — NodeType indexes only): the named node
/// type's indexes when the statement pins one, otherwise every type's, plus
/// the workspace's own.
pub(crate) async fn planner_compound_indexes<S: Storage>(
    storage: &S,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: Option<&str>,
    node_type: Option<&str>,
) -> Option<Vec<CompoundIndexDefinition>> {
    let mut all = match node_type {
        Some(name) => {
            helpers::load_compound_indexes(storage, tenant_id, repo_id, branch, name).await
        }
        None => helpers::load_all_compound_indexes(storage, tenant_id, repo_id, branch).await,
    }
    .unwrap_or_default();
    if let Some(workspace) = workspace {
        all.extend(load_workspace_compound_indexes(storage, tenant_id, repo_id, workspace).await);
    }
    if all.is_empty() {
        None
    } else {
        Some(all)
    }
}
