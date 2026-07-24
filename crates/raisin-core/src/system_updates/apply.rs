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

//! Application of a single system update (NodeType / Workspace).
//!
//! These functions are the ONE place a built-in definition is written into a
//! repository and recorded as applied. Both callers share them:
//!
//! - the admin-triggered HTTP apply endpoint
//!   (`raisin-transport-http/src/handlers/system_updates/`), and
//! - the startup resync (`raisin-server/src/startup/binary.rs`).
//!
//! Keeping them together is what makes the two paths agree on ID / timestamp
//! preservation and on recording the content hash — previously the apply logic
//! lived only in the HTTP layer, so the startup path used a different
//! (version-gated) write and the two could disagree about what was applied.

use chrono::Utc;
use raisin_error::Result;
use raisin_models::nodes::types::NodeType;
use raisin_models::workspace::Workspace;
use raisin_storage::system_updates::{AppliedDefinition, ResourceType};
use raisin_storage::{
    scope::{BranchScope, RepoScope},
    CommitMetadata, NodeTypeRepository, Storage, SystemUpdateRepository, WorkspaceRepository,
};
use std::sync::Arc;

/// Write a global NodeType into a repository and record its content hash.
///
/// Preserves the existing NodeType's `id` and `created_at` so the update is an
/// in-place schema change rather than a replacement identity.
///
/// # Arguments
/// * `content_hash` - SHA256 of the source YAML, recorded as applied
/// * `applied_by` - who applied it ("system" for the startup resync)
pub async fn apply_nodetype<S: Storage, R: SystemUpdateRepository>(
    storage: Arc<S>,
    system_update_repo: &R,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    nodetype: &NodeType,
    content_hash: &str,
    applied_by: &str,
) -> Result<()> {
    let mut nodetype = nodetype.clone();

    if nodetype.id.is_none() {
        nodetype.id = Some(nanoid::nanoid!());
    }
    if nodetype.created_at.is_none() {
        nodetype.created_at = Some(Utc::now());
    }

    // Preserve identity of the definition already in the repo.
    if let Some(existing) = storage
        .node_types()
        .get(
            BranchScope::new(tenant_id, repo_id, branch),
            &nodetype.name,
            None,
        )
        .await?
    {
        nodetype.id = existing.id;
        nodetype.created_at = existing.created_at;
    }

    nodetype.updated_at = Some(Utc::now());

    storage
        .node_types()
        .put(
            BranchScope::new(tenant_id, repo_id, branch),
            nodetype.clone(),
            CommitMetadata::system(format!("System update: {}", nodetype.name)),
        )
        .await?;

    system_update_repo
        .set_applied(
            tenant_id,
            repo_id,
            ResourceType::NodeType,
            &nodetype.name,
            AppliedDefinition {
                content_hash: content_hash.to_string(),
                applied_version: nodetype.version,
                applied_at: Utc::now(),
                applied_by: applied_by.to_string(),
            },
        )
        .await?;

    tracing::info!(
        tenant_id = %tenant_id,
        repo_id = %repo_id,
        branch = %branch,
        nodetype = %nodetype.name,
        hash = %&content_hash[..8.min(content_hash.len())],
        applied_by = %applied_by,
        "Applied NodeType system update"
    );

    Ok(())
}

/// Write a global Workspace into a repository and record its content hash.
///
/// Preserves the existing Workspace's `created_at`.
pub async fn apply_workspace<S: Storage, R: SystemUpdateRepository>(
    storage: Arc<S>,
    system_update_repo: &R,
    tenant_id: &str,
    repo_id: &str,
    workspace: &Workspace,
    content_hash: &str,
    applied_by: &str,
) -> Result<()> {
    let mut workspace = workspace.clone();

    workspace.updated_at = Some(raisin_models::StorageTimestamp::now());

    if let Some(existing) = storage
        .workspaces()
        .get(RepoScope::new(tenant_id, repo_id), &workspace.name)
        .await?
    {
        workspace.created_at = existing.created_at;
    }

    storage
        .workspaces()
        .put(RepoScope::new(tenant_id, repo_id), workspace.clone())
        .await?;

    system_update_repo
        .set_applied(
            tenant_id,
            repo_id,
            ResourceType::Workspace,
            &workspace.name,
            AppliedDefinition {
                content_hash: content_hash.to_string(),
                applied_version: None, // Workspaces have no version field
                applied_at: Utc::now(),
                applied_by: applied_by.to_string(),
            },
        )
        .await?;

    tracing::info!(
        tenant_id = %tenant_id,
        repo_id = %repo_id,
        workspace = %workspace.name,
        hash = %&content_hash[..8.min(content_hash.len())],
        applied_by = %applied_by,
        "Applied Workspace system update"
    );

    Ok(())
}
