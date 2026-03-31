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

/// Stream ordered child IDs (stub)
pub(crate) async fn stream_ordered_child_ids(
    _repo: &InMemoryNodeRepo,
    _tenant_id: &str,
    _repo_id: &str,
    _branch: &str,
    _workspace: &str,
    _parent_id: &str,
    _max_revision: Option<&HLC>,
) -> Result<Vec<String>> {
    // TODO: Implement ordered child ID streaming for in-memory storage
    Ok(Vec::new())
}
