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

//! Pending updates detection
//!
//! This module provides functionality to check for pending system updates
//! by comparing the embedded definitions with what has been applied to
//! a repository.

use super::breaking_changes::{
    detect_nodetype_breaking_changes, detect_workspace_breaking_changes,
};
use crate::nodetype_init::load_global_nodetypes_with_hashes;
use crate::package_init::load_builtin_packages_with_hashes;
use crate::workspace_init::load_global_workspaces_with_hashes;
use raisin_error::Result;
use raisin_storage::system_updates::{
    PendingUpdate, PendingUpdatesSummary, ResourceType, SystemUpdateRepository,
};
use raisin_storage::{
    scope::{BranchScope, RepoScope},
    NodeRepository, NodeTypeRepository, Storage, WorkspaceRepository,
};
use std::sync::Arc;

/// Helper struct for checking pending updates
pub struct PendingUpdatesChecker<S: Storage> {
    storage: Arc<S>,
    tenant_id: String,
    repo_id: String,
    branch: String,
}

impl<S: Storage> PendingUpdatesChecker<S> {
    /// Create a new pending updates checker
    pub fn new(storage: Arc<S>, tenant_id: &str, repo_id: &str, branch: &str) -> Self {
        Self {
            storage,
            tenant_id: tenant_id.to_string(),
            repo_id: repo_id.to_string(),
            branch: branch.to_string(),
        }
    }
}

/// Check for pending updates in a repository
///
/// This function compares the embedded NodeType and Workspace definitions
/// with what has been recorded as applied in the repository. Any definitions
/// with different content hashes are returned as pending updates.
///
/// # Arguments
/// * `storage` - Storage instance
/// * `system_update_repo` - Repository for tracking applied hashes
/// * `tenant_id` - Tenant identifier
/// * `repo_id` - Repository identifier
/// * `branch` - Branch name (for looking up current NodeTypes)
///
/// # Returns
/// A summary of pending updates including breaking change detection
pub async fn check_pending_updates<S: Storage, R: SystemUpdateRepository>(
    storage: Arc<S>,
    system_update_repo: &R,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<PendingUpdatesSummary> {
    tracing::debug!(
        tenant_id = %tenant_id,
        repo_id = %repo_id,
        branch = %branch,
        "Starting check_pending_updates"
    );

    let mut updates = Vec::new();
    let mut breaking_count = 0;

    // Check NodeTypes
    let global_nodetypes = load_global_nodetypes_with_hashes();
    tracing::debug!(count = global_nodetypes.len(), "Loaded global NodeTypes");
    for (nodetype, new_hash) in global_nodetypes {
        let applied = system_update_repo
            .get_applied(tenant_id, repo_id, ResourceType::NodeType, &nodetype.name)
            .await?;

        let (old_hash, old_version) = match &applied {
            Some(entry) => (Some(entry.content_hash.clone()), entry.applied_version),
            None => (None, None),
        };

        // Check if update is needed
        let needs_update = match &old_hash {
            Some(hash) => hash != &new_hash,
            None => true, // Never applied
        };

        if needs_update {
            // Check for breaking changes by comparing with current NodeType
            let breaking_changes = if old_hash.is_some() {
                // Get the currently applied NodeType to detect breaking changes
                if let Some(current_nodetype) = storage
                    .node_types()
                    .get(
                        BranchScope::new(tenant_id, repo_id, branch),
                        &nodetype.name,
                        None,
                    )
                    .await?
                {
                    detect_nodetype_breaking_changes(&current_nodetype, &nodetype)
                } else {
                    vec![]
                }
            } else {
                vec![] // New NodeType, no breaking changes possible
            };

            let is_breaking = !breaking_changes.is_empty();
            if is_breaking {
                breaking_count += 1;
            }

            updates.push(PendingUpdate {
                resource_type: ResourceType::NodeType,
                name: nodetype.name.clone(),
                new_hash,
                old_hash,
                new_version: nodetype.version,
                old_version,
                is_breaking,
                breaking_changes,
            });
        }
    }

    // Check Workspaces
    let global_workspaces = load_global_workspaces_with_hashes();
    tracing::debug!(count = global_workspaces.len(), "Loaded global Workspaces");
    for (workspace, new_hash) in global_workspaces {
        let applied = system_update_repo
            .get_applied(tenant_id, repo_id, ResourceType::Workspace, &workspace.name)
            .await?;

        let (old_hash, old_version) = match &applied {
            Some(entry) => (Some(entry.content_hash.clone()), entry.applied_version),
            None => (None, None),
        };

        // Check if update is needed
        let needs_update = match &old_hash {
            Some(hash) => hash != &new_hash,
            None => true, // Never applied
        };

        if needs_update {
            // Check for breaking changes by comparing with current Workspace
            let breaking_changes = if old_hash.is_some() {
                // Get the currently applied Workspace to detect breaking changes
                if let Some(current_workspace) = storage
                    .workspaces()
                    .get(RepoScope::new(tenant_id, repo_id), &workspace.name)
                    .await?
                {
                    detect_workspace_breaking_changes(&current_workspace, &workspace)
                } else {
                    vec![]
                }
            } else {
                vec![] // New Workspace, no breaking changes possible
            };

            let is_breaking = !breaking_changes.is_empty();
            if is_breaking {
                breaking_count += 1;
            }

            updates.push(PendingUpdate {
                resource_type: ResourceType::Workspace,
                name: workspace.name.clone(),
                new_hash,
                old_hash,
                // Workspaces don't have a version field, so we use None
                new_version: None,
                old_version,
                is_breaking,
                breaking_changes,
            });
        }
    }

    // Check Builtin Packages
    let builtin_packages = load_builtin_packages_with_hashes();
    tracing::debug!(count = builtin_packages.len(), "Loaded builtin Packages");
    for package_info in builtin_packages {
        tracing::debug!(
            package = %package_info.manifest.name,
            new_hash = %&package_info.content_hash[..8],
            "Checking package"
        );

        let applied = system_update_repo
            .get_applied(
                tenant_id,
                repo_id,
                ResourceType::Package,
                &package_info.manifest.name,
            )
            .await?;

        let (old_hash, old_version) = match &applied {
            Some(entry) => (Some(entry.content_hash.clone()), entry.applied_version),
            None => (None, None),
        };

        tracing::debug!(
            package = %package_info.manifest.name,
            has_applied = old_hash.is_some(),
            old_hash = ?old_hash.as_ref().map(|h| &h[..8]),
            "Package applied status"
        );

        // Check if update is needed
        let needs_update = match &old_hash {
            Some(hash) => hash != &package_info.content_hash,
            None => true, // Never applied
        };

        tracing::debug!(
            package = %package_info.manifest.name,
            needs_update = needs_update,
            "Package update check result"
        );

        if needs_update {
            // Packages don't have breaking changes - they only add content
            updates.push(PendingUpdate {
                resource_type: ResourceType::Package,
                name: package_info.manifest.name.clone(),
                new_hash: package_info.content_hash,
                old_hash,
                new_version: None, // Package version is a string, not i32
                old_version,
                is_breaking: false, // Packages don't have breaking changes
                breaking_changes: vec![],
            });
        }
    }

    let total_pending = updates.len();

    tracing::debug!(
        total_pending = total_pending,
        breaking_count = breaking_count,
        has_updates = total_pending > 0,
        "check_pending_updates complete"
    );

    Ok(PendingUpdatesSummary {
        has_updates: total_pending > 0,
        total_pending,
        breaking_count,
        updates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_global_nodetypes_with_hashes() {
        let nodetypes = load_global_nodetypes_with_hashes();
        assert!(!nodetypes.is_empty(), "Should load at least one NodeType");

        for (nt, hash) in &nodetypes {
            assert!(!nt.name.is_empty(), "NodeType should have a name");
            assert_eq!(hash.len(), 64, "Hash should be 64 hex chars");
        }
    }

    #[test]
    fn test_load_global_workspaces_with_hashes() {
        let workspaces = load_global_workspaces_with_hashes();
        assert!(!workspaces.is_empty(), "Should load at least one Workspace");

        for (ws, hash) in &workspaces {
            assert!(!ws.name.is_empty(), "Workspace should have a name");
            assert_eq!(hash.len(), 64, "Hash should be 64 hex chars");
        }
    }
}
