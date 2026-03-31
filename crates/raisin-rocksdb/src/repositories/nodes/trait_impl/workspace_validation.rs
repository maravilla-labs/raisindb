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

//! Workspace and parent-child validation helpers.
//!
//! These contain the actual validation logic that the `NodeRepository` trait
//! dispatcher delegates to.  Keeping them in a separate module ensures the
//! main trait impl stays under the 500-line limit.

use raisin_error::Result;

use crate::repositories::nodes::NodeRepositoryImpl;

impl NodeRepositoryImpl {
    /// Check whether `parent_node_type` allows `child_node_type` as a child.
    ///
    /// Rules:
    /// 1. If the parent `NodeType` does not exist, validation is skipped.
    /// 2. Empty `allowed_children` or a `"*"` entry means all types are allowed.
    /// 3. Otherwise `child_node_type` must appear in `allowed_children`.
    pub(crate) async fn validate_parent_allows_child_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        parent_node_type: &str,
        child_node_type: &str,
    ) -> Result<()> {
        use raisin_storage::NodeTypeRepository;

        let parent_type = match self
            .node_type_repo
            .get(
                raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
                parent_node_type,
                None,
            )
            .await?
        {
            Some(node_type) => node_type,
            None => return Ok(()),
        };

        if parent_type.allowed_children.is_empty() {
            return Ok(());
        }

        if parent_type.allowed_children.contains(&"*".to_string()) {
            return Ok(());
        }

        if parent_type
            .allowed_children
            .contains(&child_node_type.to_string())
        {
            return Ok(());
        }

        Err(raisin_error::Error::Validation(format!(
            "Node type '{}' is not allowed as a child of '{}'. Allowed children: {:?}",
            child_node_type, parent_node_type, parent_type.allowed_children
        )))
    }

    /// Check whether `workspace` allows `node_type` (and optionally as a root node).
    ///
    /// Rules:
    /// 1. If the workspace config does not exist, validation is skipped.
    /// 2. `allowed_node_types` restricts all nodes; `"*"` or empty means all allowed.
    /// 3. For root nodes, `allowed_root_node_types` is additionally checked.
    pub(crate) async fn validate_workspace_allows_node_type_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        workspace: &str,
        node_type: &str,
        is_root_node: bool,
    ) -> Result<()> {
        use raisin_storage::WorkspaceRepository;

        let workspace_config = match self
            .workspace_repo
            .get(
                raisin_storage::RepoScope::new(tenant_id, repo_id),
                workspace,
            )
            .await?
        {
            Some(ws) => ws,
            None => return Ok(()),
        };

        // Rule 1: Check allowed_node_types (applies to ALL nodes)
        if !workspace_config.allowed_node_types.is_empty()
            && !workspace_config
                .allowed_node_types
                .contains(&"*".to_string())
            && !workspace_config
                .allowed_node_types
                .contains(&node_type.to_string())
        {
            return Err(raisin_error::Error::Validation(format!(
                "Node type '{}' is not allowed in workspace '{}'. Allowed node types: {:?}",
                node_type, workspace, workspace_config.allowed_node_types
            )));
        }

        // Rule 2: For root nodes, also check allowed_root_node_types
        if is_root_node
            && !workspace_config.allowed_root_node_types.is_empty()
            && !workspace_config
                .allowed_root_node_types
                .contains(&"*".to_string())
            && !workspace_config
                .allowed_root_node_types
                .contains(&node_type.to_string())
        {
            return Err(raisin_error::Error::Validation(format!(
                "Node type '{}' is not allowed as a root node in workspace '{}'. Allowed root node types: {:?}",
                node_type, workspace, workspace_config.allowed_root_node_types
            )));
        }

        Ok(())
    }
}
