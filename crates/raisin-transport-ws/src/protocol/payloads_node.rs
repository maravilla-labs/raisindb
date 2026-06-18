// SPDX-License-Identifier: BSL-1.1

//! Payload types for node-related operations.
//!
//! Covers CRUD, manipulation (move/rename/copy/reorder), tree traversal,
//! property-path access, and relationship management.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Node CRUD payloads
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeCreatePayload {
    pub node_type: String,
    pub path: String,
    pub properties: HashMap<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<serde_json::Value>,
}

/// Create (or upsert) a node, auto-creating any missing ancestor folders.
///
/// `path` is the full node path; missing ancestors are created as
/// `parent_node_type` (defaults to `raisin:Folder`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeCreateDeepPayload {
    pub node_type: String,
    pub path: String,
    pub properties: HashMap<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_node_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeUpdatePayload {
    pub node_id: String,
    pub properties: HashMap<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeDeletePayload {
    pub node_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeGetPayload {
    pub node_id: String,
}

/// Request a node's revision history (git-style "file history"), newest first.
///
/// Identify the node by either `node_id` or `path` (at least one required).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeHistoryPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Maximum number of revisions to return (newest first). Unbounded if omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

/// Query a node's audit-log entries.
///
/// Identify the node by either `node_id` or `path` (at least one required).
/// Audit logs are only produced for NodeTypes marked `auditable`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditQueryPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeQueryPayload {
    pub query: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqlQueryPayload {
    /// SQL query string
    pub query: String,

    /// Parameters for the query (for injection protection)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Vec<serde_json::Value>>,
}

// ---------------------------------------------------------------------------
// Node manipulation payloads
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeMovePayload {
    pub from_path: String,
    pub to_parent_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeRenamePayload {
    pub old_path: String,
    pub new_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeCopyPayload {
    pub source_path: String,
    pub target_parent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeCopyTreePayload {
    pub source_path: String,
    pub target_parent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeReorderPayload {
    pub parent_path: String,
    pub child_name: String,
    pub position: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeMoveChildBeforePayload {
    pub parent_path: String,
    pub child_name: String,
    pub before_child_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeMoveChildAfterPayload {
    pub parent_path: String,
    pub child_name: String,
    pub after_child_name: String,
}

/// Replay a parent's child order from `source_branch` onto the request's
/// (target) branch — the cross-branch ordering primitive used by selective
/// publish flows. The target branch + workspace come from the request context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeApplyChildOrderPayload {
    pub parent_path: String,
    pub source_branch: String,
}

// ---------------------------------------------------------------------------
// Tree operation payloads
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeListChildrenPayload {
    pub parent_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeGetTreePayload {
    pub parent_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeGetTreeFlatPayload {
    pub parent_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<u32>,
}

// ---------------------------------------------------------------------------
// Property path operation payloads
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropertyGetPayload {
    pub node_path: String,
    pub property_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropertyUpdatePayload {
    pub node_path: String,
    pub property_path: String,
    pub value: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Relationship operation payloads
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationAddPayload {
    pub source_path: String,
    pub target_workspace: String,
    pub target_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationRemovePayload {
    pub source_path: String,
    pub target_workspace: String,
    pub target_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationsGetPayload {
    pub node_path: String,
}
