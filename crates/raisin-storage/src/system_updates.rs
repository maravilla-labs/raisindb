// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! System updates tracking for built-in NodeTypes and Workspaces
//!
//! This module provides types and traits for tracking which versions of
//! built-in NodeTypes and Workspaces have been applied to each repository.
//! This enables detecting when server updates contain new/changed definitions
//! and allowing administrators to review and apply them.

use chrono::{DateTime, Utc};
use raisin_error::Result;
use serde::{Deserialize, Serialize};

/// Type of system resource (NodeType, Workspace, or Package)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceType {
    /// A NodeType definition
    NodeType,
    /// A Workspace definition
    Workspace,
    /// A builtin package
    Package,
}

impl std::fmt::Display for ResourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResourceType::NodeType => write!(f, "nodetype"),
            ResourceType::Workspace => write!(f, "workspace"),
            ResourceType::Package => write!(f, "package"),
        }
    }
}

impl std::str::FromStr for ResourceType {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "nodetype" | "node_type" => Ok(ResourceType::NodeType),
            "workspace" => Ok(ResourceType::Workspace),
            "package" => Ok(ResourceType::Package),
            _ => Err(format!("Unknown resource type: {}", s)),
        }
    }
}

/// Record of an applied definition (NodeType or Workspace)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppliedDefinition {
    /// SHA256 hash of the YAML file content when applied
    pub content_hash: String,
    /// Version field from the YAML file when applied (if present)
    pub applied_version: Option<i32>,
    /// When this definition was applied
    pub applied_at: DateTime<Utc>,
    /// Who applied it ("system" for auto-init, or admin username)
    pub applied_by: String,
}

/// Type of breaking change detected
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BreakingChangeType {
    /// A property was removed from the NodeType
    PropertyRemoved,
    /// A property's type was changed
    PropertyTypeChanged,
    /// A required constraint was added to an existing property
    RequiredAdded,
    /// An allowed child type was removed from allowed_children
    AllowedChildrenRemoved,
    /// A mixin was removed from the NodeType
    MixinRemoved,
    /// An allowed node type was removed from the Workspace
    AllowedNodeTypeRemoved,
    /// An allowed root node type was removed from the Workspace
    AllowedRootNodeTypeRemoved,
}

impl std::fmt::Display for BreakingChangeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BreakingChangeType::PropertyRemoved => write!(f, "property_removed"),
            BreakingChangeType::PropertyTypeChanged => write!(f, "property_type_changed"),
            BreakingChangeType::RequiredAdded => write!(f, "required_added"),
            BreakingChangeType::AllowedChildrenRemoved => write!(f, "allowed_children_removed"),
            BreakingChangeType::MixinRemoved => write!(f, "mixin_removed"),
            BreakingChangeType::AllowedNodeTypeRemoved => write!(f, "allowed_node_type_removed"),
            BreakingChangeType::AllowedRootNodeTypeRemoved => {
                write!(f, "allowed_root_node_type_removed")
            }
        }
    }
}

/// A single breaking change detected between old and new definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreakingChange {
    /// Type of breaking change
    pub change_type: BreakingChangeType,
    /// Human-readable description of the change
    pub description: String,
    /// Path to the affected element (e.g., "properties.title", "allowed_children")
    pub path: String,
}

/// A pending update that can be applied to a repository
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingUpdate {
    /// Type of resource (NodeType or Workspace)
    pub resource_type: ResourceType,
    /// Name of the resource (e.g., "raisin:Folder", "default")
    pub name: String,
    /// New content hash from the server binary
    pub new_hash: String,
    /// Previously applied hash (None if never applied)
    pub old_hash: Option<String>,
    /// New version from the server binary (if present in YAML)
    pub new_version: Option<i32>,
    /// Previously applied version (if present)
    pub old_version: Option<i32>,
    /// Whether this update contains breaking changes
    pub is_breaking: bool,
    /// List of breaking changes (empty if is_breaking is false)
    pub breaking_changes: Vec<BreakingChange>,
}

/// Summary of pending updates for a repository
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingUpdatesSummary {
    /// Whether there are any pending updates
    pub has_updates: bool,
    /// Total number of pending updates
    pub total_pending: usize,
    /// Number of updates that contain breaking changes
    pub breaking_count: usize,
    /// List of all pending updates
    pub updates: Vec<PendingUpdate>,
}

/// Result of applying system updates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyResult {
    /// Number of updates successfully applied
    pub applied_count: usize,
    /// Number of updates skipped (e.g., due to breaking changes without force)
    pub skipped_count: usize,
    /// Number of updates that failed
    pub failed_count: usize,
    /// Details of each update attempt
    pub details: Vec<ApplyResultDetail>,
}

/// Detail of a single update attempt
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyResultDetail {
    /// Resource type
    pub resource_type: ResourceType,
    /// Resource name
    pub name: String,
    /// Whether the update was applied successfully
    pub success: bool,
    /// Error message if failed
    pub error: Option<String>,
    /// Whether it was skipped due to breaking changes
    pub skipped_breaking: bool,
}

/// Repository trait for system update tracking
///
/// Implementations store and retrieve records of which definition versions
/// have been applied to each repository.
#[async_trait::async_trait]
pub trait SystemUpdateRepository: Send + Sync {
    /// Get the applied definition record for a resource
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `resource_type` - Type of resource (NodeType or Workspace)
    /// * `name` - Name of the resource
    ///
    /// # Returns
    /// * `Ok(Some(record))` - The applied definition record
    /// * `Ok(None)` - Resource has never been applied to this repository
    async fn get_applied(
        &self,
        tenant_id: &str,
        repo_id: &str,
        resource_type: ResourceType,
        name: &str,
    ) -> Result<Option<AppliedDefinition>>;

    /// Record that a definition was applied to a repository
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `resource_type` - Type of resource (NodeType or Workspace)
    /// * `name` - Name of the resource
    /// * `entry` - The applied definition record
    async fn set_applied(
        &self,
        tenant_id: &str,
        repo_id: &str,
        resource_type: ResourceType,
        name: &str,
        entry: AppliedDefinition,
    ) -> Result<()>;

    /// List all applied definitions for a repository
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    ///
    /// # Returns
    /// Vector of (resource_type, name, record) tuples
    async fn list_applied(
        &self,
        tenant_id: &str,
        repo_id: &str,
    ) -> Result<Vec<(ResourceType, String, AppliedDefinition)>>;

    /// Delete an applied definition record
    ///
    /// Used when a built-in resource is removed from the server.
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `resource_type` - Type of resource (NodeType or Workspace)
    /// * `name` - Name of the resource
    async fn delete_applied(
        &self,
        tenant_id: &str,
        repo_id: &str,
        resource_type: ResourceType,
        name: &str,
    ) -> Result<()>;
}
