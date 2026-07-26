//! Stub/placeholder implementations for in-memory node repository
//!
//! These methods return empty results or errors because they are either
//! not yet implemented for the in-memory backend or are only used
//! in the RocksDB backend for SQL query optimization.

use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use raisin_models as models;
use raisin_storage::ListOptions;
use std::collections::HashMap;

use super::InMemoryNodeRepo;

/// Create a deep node (not supported in in-memory backend)
pub(crate) async fn create_deep_node(
    _repo: &InMemoryNodeRepo,
    _tenant_id: &str,
    _repo_id: &str,
    _branch: &str,
    _workspace: &str,
    _path: &str,
    _node: models::nodes::Node,
    _parent_node_type: &str,
) -> Result<models::nodes::Node> {
    Err(Error::Backend(
        "create_deep_node is not supported in the in-memory storage backend".to_string(),
    ))
}

/// Move node tree (not supported in in-memory backend)
pub(crate) async fn move_node_tree(
    _repo: &InMemoryNodeRepo,
    _tenant_id: &str,
    _repo_id: &str,
    _branch: &str,
    _workspace: &str,
    _id: &str,
    _new_path: &str,
) -> Result<()> {
    Err(Error::Backend(
        "move_node_tree is not supported in the in-memory storage backend".to_string(),
    ))
}

/// Scan by path prefix (stub)
pub(crate) async fn scan_by_path_prefix(
    _repo: &InMemoryNodeRepo,
    _tenant_id: &str,
    _repo_id: &str,
    _branch: &str,
    _workspace: &str,
    _path_prefix: &str,
    _options: ListOptions,
) -> Result<Vec<models::nodes::Node>> {
    // TODO: Implement efficient path prefix scanning for memory storage
    Ok(Vec::new())
}

/// Scan descendants ordered (stub)
pub(crate) async fn scan_descendants_ordered(
    _repo: &InMemoryNodeRepo,
    _tenant_id: &str,
    _repo_id: &str,
    _branch: &str,
    _workspace: &str,
    _parent_node_id: &str,
    _options: ListOptions,
) -> Result<Vec<models::nodes::Node>> {
    // TODO: Implement ordered descendants scan for in-memory storage
    Ok(Vec::new())
}

/// Get descendants bulk (stub)
pub(crate) async fn get_descendants_bulk(
    _repo: &InMemoryNodeRepo,
    _tenant_id: &str,
    _repo_id: &str,
    _branch: &str,
    _workspace: &str,
    _parent_path: &str,
    _max_depth: u32,
    _max_revision: Option<&HLC>,
) -> Result<HashMap<String, models::nodes::Node>> {
    // TODO: Implement bulk descendant fetching for memory storage
    Ok(HashMap::new())
}

/// Validate parent allows child (stub - permissive mode)
pub(crate) async fn validate_parent_allows_child(
    _repo: &InMemoryNodeRepo,
    _tenant_id: &str,
    _repo_id: &str,
    _branch: &str,
    _parent_node_type: &str,
    _child_node_type: &str,
) -> Result<()> {
    // TODO: Implement allowed_children validation for in-memory storage
    Ok(())
}

/// Validate workspace allows node type (stub - permissive mode)
pub(crate) async fn validate_workspace_allows_node_type(
    _repo: &InMemoryNodeRepo,
    _tenant_id: &str,
    _repo_id: &str,
    _workspace: &str,
    _node_type: &str,
    _is_root_node: bool,
) -> Result<()> {
    // TODO: Implement workspace allowed_node_types validation for in-memory storage
    Ok(())
}

/// Stream ordered child IDs, in the parent's editorial order.
pub(crate) async fn stream_ordered_child_ids(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    parent_id: &str,
    max_revision: Option<&HLC>,
) -> Result<Vec<String>> {
    Ok(list_ordered_children_page(
        repo,
        tenant_id,
        repo_id,
        branch,
        workspace,
        parent_id,
        None,
        None,
        false,
        max_revision,
    )
    .await?
    .into_iter()
    .map(|child| child.child_id)
    .collect())
}

/// Page through a parent's children in editorial order.
///
/// The in-memory backend has no fractional index — it keeps order as the
/// parent's `children` vector of names (see `nodes::reorder`). Order labels are
/// therefore synthesized from the child's position, zero-padded so they sort
/// lexicographically like the real labels do. That is enough for the trait
/// contract and for keyset paging within one snapshot, but unlike the RocksDB
/// backend the labels are **not stable across reorders** — a reorder shifts
/// every subsequent label. Tests that need reorder-stable cursors must use the
/// RocksDB backend.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn list_ordered_children_page(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    parent_id: &str,
    after_label: Option<&str>,
    limit: Option<usize>,
    descending: bool,
    _max_revision: Option<&HLC>,
) -> Result<Vec<raisin_storage::OrderedChild>> {
    if limit == Some(0) {
        return Ok(Vec::new());
    }

    let workspace_prefix = crate::NodeKey::workspace_prefix(tenant_id, repo_id, branch, workspace);
    let map = repo.nodes.read().await;

    // Root-level children are addressed as parent_id "/". They are only held in a
    // parent's `children` vector when an explicit "/" node exists; otherwise they
    // are identified by having no parent, and ordered by name for determinism
    // (matching `tree_ops::list_root`).
    let in_workspace = |key: &String| key.starts_with(&workspace_prefix);

    let parent = map
        .iter()
        .filter(|(key, _)| in_workspace(key))
        .map(|(_, node)| node)
        .find(|node| {
            if parent_id == "/" {
                node.path == "/"
            } else {
                node.id == parent_id
            }
        });

    let ordered_names: Vec<(String, String)> = match parent {
        // Resolve the parent's ordered child names to (name, path) pairs.
        Some(parent) => {
            let parent_path = parent.path.trim_end_matches('/');
            parent
                .children
                .iter()
                .map(|name| (name.clone(), format!("{}/{}", parent_path, name)))
                .collect()
        }
        // No explicit "/" node: root-level children are those at depth 1.
        //
        // Detect that from the path rather than from `parent`, which is Some("/")
        // for root-level nodes on some write paths and None on others. Ordered by
        // name for determinism, since there is no parent vector to carry an order.
        None if parent_id == "/" => {
            let mut roots: Vec<&models::nodes::Node> = map
                .iter()
                .filter(|(key, node)| {
                    in_workspace(key) && node.path.len() > 1 && node.path.matches('/').count() == 1
                })
                .map(|(_, node)| node)
                .collect();
            roots.sort_by(|a, b| a.name.cmp(&b.name));
            roots
                .into_iter()
                .map(|node| (node.name.clone(), node.path.clone()))
                .collect()
        }
        None => return Ok(Vec::new()),
    };

    let mut ordered: Vec<raisin_storage::OrderedChild> = ordered_names
        .into_iter()
        .enumerate()
        .filter_map(|(index, (child_name, child_path))| {
            let child = map
                .iter()
                .filter(|(key, _)| in_workspace(key))
                .map(|(_, node)| node)
                .find(|node| node.path == child_path)?;
            Some(raisin_storage::OrderedChild {
                child_id: child.id.clone(),
                order_label: format!("{index:08}"),
                name: child_name,
            })
        })
        .collect();

    if descending {
        ordered.reverse();
    }
    if let Some(cursor) = after_label {
        ordered.retain(|child| {
            if descending {
                child.order_label.as_str() < cursor
            } else {
                child.order_label.as_str() > cursor
            }
        });
    }
    if let Some(limit) = limit {
        ordered.truncate(limit);
    }

    Ok(ordered)
}
