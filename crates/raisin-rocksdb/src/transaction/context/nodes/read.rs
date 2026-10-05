//! Node read operations
//!
//! This module contains the implementation of node read operations for transactions:
//! - `get_node`: Get a node by ID with read-your-writes semantics
//! - `get_node_by_path`: Get a node by path with read-your-writes semantics
//!
//! # Key Features
//!
//! ## Read-Your-Writes Semantics
//!
//! All read operations check the in-memory cache first, ensuring that uncommitted
//! changes made earlier in the transaction are visible to later operations.
//!
//! ## MVCC Read
//!
//! Reads the latest version of the node at or before the branch HEAD.
//! Skips tombstone markers to respect deletions.
//!
//! ## StorageNode Compatibility
//!
//! Supports both old (Node with path) and new (StorageNode without path) formats.
//! Path is materialized from NODE_PATH index when needed.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_storage::BranchRepository;
use rocksdb::DB;
use std::sync::Arc;

use crate::transaction::types::is_tombstone;
use crate::transaction::RocksDBTransaction;
use crate::{cf, cf_handle, keys};

/// Deserialize node with path materialization support — the shared decoder
/// (`crate::mvcc_read::deserialize_node_with_path`), kept under this name so
/// the readers below read as before.
#[allow(clippy::too_many_arguments)]
fn deserialize_node_with_path(
    db: &Arc<DB>,
    bytes: &[u8],
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    read_at: &HLC,
    blob_revision: &HLC,
) -> Result<Node> {
    crate::mvcc_read::deserialize_node_with_path(
        db,
        bytes,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        read_at,
        blob_revision,
    )
}

/// Get a node by ID with read-your-writes semantics
///
/// Checks the read cache first to ensure uncommitted changes are visible.
///
/// # MVCC Read
///
/// Reads the latest version of the node at or before the branch HEAD.
/// Skips tombstone markers to respect deletions.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `workspace` - The workspace containing the node
/// * `node_id` - The ID of the node to read
///
/// # Returns
///
/// Ok(Some(node)) if found, Ok(None) if not found or deleted
pub async fn get_node(
    tx: &RocksDBTransaction,
    workspace: &str,
    node_id: &str,
) -> Result<Option<Node>> {
    get_node_bounded(tx, workspace, node_id, true).await
}

/// Resolve a node by ID **ignoring the branch HEAD bound**.
///
/// Identical to [`get_node`] (same read-your-writes cache, same tombstone
/// handling, same RLS check) except that it does not skip revisions newer
/// than the branch HEAD.
///
/// # Why this exists
///
/// A node whose latest revision sits ABOVE the branch HEAD ("stranded") is
/// invisible to every HEAD-bounded read, but it still *occupies* its id and
/// path: the create-time uniqueness checks in
/// `NodeRepositoryImpl::validate_for_create` resolve the latest revision
/// unbounded and reject the write as a Conflict. A HEAD-bounded probe
/// therefore routes such a write to CREATE, which then always fails — and
/// the node can never self-heal, because the only write that would advance
/// HEAD past it is the write that keeps failing.
///
/// Callers deciding CREATE-vs-UPDATE must resolve identity over the branch's
/// whole revision history, not just its HEAD-visible prefix. **This is not a
/// read API** — never use it to serve reads, or committed-but-not-yet-visible
/// revisions leak into query results.
pub async fn get_node_ignoring_head(
    tx: &RocksDBTransaction,
    workspace: &str,
    node_id: &str,
) -> Result<Option<Node>> {
    get_node_bounded(tx, workspace, node_id, false).await
}

async fn get_node_bounded(
    tx: &RocksDBTransaction,
    workspace: &str,
    node_id: &str,
    bound_to_head: bool,
) -> Result<Option<Node>> {
    // Check read cache first for read-your-writes semantics. A node this
    // transaction WROTE is served as is; one it only MOVED (committed state,
    // new path) replaces the committed read below but still passes RLS.
    let moved = {
        let cache = tx
            .read_cache
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        let cache_key = (workspace.to_string(), node_id.to_string());
        if let Some(cached) = cache.nodes.get(&cache_key) {
            return Ok(cached.clone());
        }
        cache.moved_nodes.get(&cache_key).cloned()
    };

    // 1. Get metadata
    let (tenant_id, repo_id, branch) = {
        let meta = tx
            .metadata
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        (
            meta.tenant_id.clone(),
            meta.repo_id.clone(),
            meta.branch.clone().ok_or_else(|| {
                raisin_error::Error::Validation("Branch not set in transaction".into())
            })?,
        )
    };

    // 2. Get HEAD revision
    let head_revision = tx
        .branch_repo
        .get_branch(&tenant_id, &repo_id, &branch)
        .await?
        .ok_or_else(|| raisin_error::Error::NotFound(format!("Branch {} not found", branch)))?
        .head;

    let node = match moved {
        Some(node) => node,
        None => match read_committed(
            tx,
            &tenant_id,
            &repo_id,
            &branch,
            workspace,
            node_id,
            bound_to_head.then_some(&head_revision),
        )? {
            Some(node) => node,
            None => return Ok(None),
        },
    };

    // RLS check - SECURITY: deny-by-default if no auth context.
    // Clone the auth context out of the metadata guard so the guard is
    // released before the async graph-resolver evaluation below.
    let auth_opt = {
        let meta = tx
            .metadata
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        meta.auth_context.clone()
    };
    match auth_opt {
        Some(auth) => {
            use raisin_core::services::rls_filter;
            use raisin_models::permissions::{Operation, PermissionScope};
            use raisin_storage::{scope::BranchScope, Storage};

            let (tid, rid, br): (&str, &str, &str) = (&tenant_id, &repo_id, &branch);
            let scope = PermissionScope::new(workspace, br);
            // Fast path: only build the cache-backed graph resolver when a
            // permission actually carries a `RELATES … VIA` condition;
            // otherwise evaluate synchronously with no per-read allocation.
            let allowed = if auth.uses_graph_rls() {
                let resolver = tx
                    .storage
                    .graph_resolver(BranchScope::new(tid, rid, br), &head_revision);
                rls_filter::can_perform_async(
                    &node,
                    Operation::Read,
                    &auth,
                    &scope,
                    resolver.as_deref(),
                )
                .await
            } else {
                rls_filter::can_perform(&node, Operation::Read, &auth, &scope)
            };

            if !allowed {
                tracing::debug!(
                    node_id = %node_id,
                    workspace = %workspace,
                    "RLS: denying read access to node"
                );
                return Ok(None);
            }
        }
        None => {
            // SECURITY: Deny read if no auth context set on transaction
            tracing::warn!(
                node_id = %node_id,
                workspace = %workspace,
                "Transaction has no auth context - denying get_node read"
            );
            return Ok(None);
        }
    }

    Ok(Some(node))
}

/// The committed record of `node_id`: the newest version at or before
/// `max_revision` (`None`: newest), with its path as of that read.
fn read_committed(
    tx: &RocksDBTransaction,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    max_revision: Option<&HLC>,
) -> Result<Option<Node>> {
    // The newest version at or before HEAD: one seek to `{prefix}{~HEAD}`
    // (see `crate::mvcc_read`), bounded to this node's prefix so a lookup for a
    // NONEXISTENT id can never read the next node in the keyspace (observed
    // once: put_node with a fresh id taking the UPDATE branch against an
    // unrelated node). `max_revision == None` is the deliberate
    // identity-resolution path (see `get_node_ignoring_head`), which must see
    // stranded revisions, so it takes the newest version unbounded.
    let cf_nodes = cf_handle(&tx.db, cf::NODES)?;
    let prefix = keys::node_key_prefix(tenant_id, repo_id, branch, workspace, node_id);
    let Some((blob_revision, value)) =
        crate::mvcc_read::newest_at_or_before(&tx.db, cf_nodes, &prefix, max_revision)?
    else {
        return Ok(None);
    };

    // A tombstone means the node is deleted.
    if is_tombstone(&value) {
        return Ok(None);
    }

    // Deserialize with StorageNode/Node compatibility. The path is the one
    // AS OF THE READ (HEAD, or the newest when unbounded), not as of the blob:
    // a later ancestor move writes NODE_PATH above the blob's revision, and
    // reading it at the blob's revision handed `move_node_tree` the pre-move
    // path — so it could not find the old parent and left the old
    // ORDERED_CHILDREN entry live (the node listed under both parents).
    let path_at = max_revision.copied().unwrap_or(crate::mvcc_read::NEWEST);
    deserialize_node_with_path(
        &tx.db,
        &value,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        &path_at,
        &blob_revision,
    )
    .map(Some)
}

/// Get a node by path with read-your-writes semantics
///
/// Checks the read cache first to ensure uncommitted changes are visible.
///
/// # Path Resolution
///
/// 1. Queries PATH_INDEX to get node_id
/// 2. Calls get_node to read the node data
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `workspace` - The workspace containing the node
/// * `path` - The path of the node to read
///
/// # Returns
///
/// Ok(Some(node)) if found, Ok(None) if not found or deleted
pub async fn get_node_by_path(
    tx: &RocksDBTransaction,
    workspace: &str,
    path: &str,
) -> Result<Option<Node>> {
    get_node_by_path_bounded(tx, workspace, path, true).await
}

/// Resolve a node by PATH **ignoring the branch HEAD bound**.
///
/// The by-path sibling of [`get_node_ignoring_head`] — see its doc comment for
/// why identity resolution must be unbounded while reads must not be. **This is
/// not a read API.**
pub async fn get_node_by_path_ignoring_head(
    tx: &RocksDBTransaction,
    workspace: &str,
    path: &str,
) -> Result<Option<Node>> {
    get_node_by_path_bounded(tx, workspace, path, false).await
}

async fn get_node_by_path_bounded(
    tx: &RocksDBTransaction,
    workspace: &str,
    path: &str,
    bound_to_head: bool,
) -> Result<Option<Node>> {
    // Check read cache first for read-your-writes semantics
    let cached_node_id = {
        let cache = tx
            .read_cache
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        let cache_key = (workspace.to_string(), path.to_string());
        cache.paths.get(&cache_key).cloned()
    }; // Drop lock here before async call

    if let Some(node_id_opt) = cached_node_id {
        if let Some(node_id) = node_id_opt {
            // Path found, now get the node (which will also check cache)
            return get_node_bounded(tx, workspace, &node_id, bound_to_head).await;
        } else {
            // Path was explicitly deleted in this transaction
            return Ok(None);
        }
    }

    // 1. Get metadata
    let (tenant_id, repo_id, branch) = {
        let meta = tx
            .metadata
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        (
            meta.tenant_id.clone(),
            meta.repo_id.clone(),
            meta.branch.clone().ok_or_else(|| {
                raisin_error::Error::Validation("Branch not set in transaction".into())
            })?,
        )
    };

    // 2. Get HEAD revision for filtering PATH_INDEX entries
    // CRITICAL: This ensures we only see nodes that are visible at the current HEAD
    let head_revision = tx
        .branch_repo
        .get_branch(&tenant_id, &repo_id, &branch)
        .await?
        .ok_or_else(|| raisin_error::Error::NotFound(format!("Branch {} not found", branch)))?
        .head;

    // 3. Query path index to get node_id: the newest entry at or before HEAD,
    // found with one seek (see `crate::mvcc_read`). Unbounded for the
    // identity-resolution path (see `get_node_by_path_ignoring_head`).
    let cf_path = cf_handle(&tx.db, cf::PATH_INDEX)?;
    let prefix = keys::path_index_key_prefix(&tenant_id, &repo_id, &branch, workspace, path);
    let max_revision = bound_to_head.then_some(&head_revision);

    tracing::debug!(
        "TX get_node_by_path: workspace={}, path={}, head_revision={}",
        workspace,
        path,
        head_revision
    );

    let Some((revision, value)) =
        crate::mvcc_read::newest_at_or_before(&tx.db, cf_path, &prefix, max_revision)?
    else {
        tracing::debug!("TX get_node_by_path: no node found for path={}", path);
        return Ok(None);
    };

    // Check for tombstone - path was deleted, return None
    if is_tombstone(&value) {
        tracing::debug!(
            "TX get_node_by_path: tombstone found for path={}, node is deleted",
            path
        );
        return Ok(None);
    }

    // Found the node ID
    let node_id = String::from_utf8(value)
        .map_err(|e| raisin_error::Error::storage(format!("Invalid node ID: {}", e)))?;

    tracing::debug!(
        "TX get_node_by_path: found node_id={} at revision={}",
        node_id,
        revision
    );

    // Now get the actual node
    get_node_bounded(tx, workspace, &node_id, bound_to_head).await
}
