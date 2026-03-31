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

//! CRUD trait method bodies for `NodeRepository`.
//!
//! These methods are called directly from the trait impl in `mod.rs`.
//! They contain the small amount of dispatch logic needed before
//! delegating to the real `_impl` methods in the `crud` submodule.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_storage::{
    CreateNodeOptions, DeleteNodeOptions, ListOptions, NodeRepository, NodeWithPopulatedChildren,
    StorageScope, UpdateNodeOptions,
};

use crate::repositories::nodes::NodeRepositoryImpl;

impl NodeRepositoryImpl {
    /// Dispatch for `NodeRepository::get` -- resolves HEAD if no revision given.
    pub(crate) async fn dispatch_get(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        id: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<Node>> {
        let target_revision = if let Some(rev) = max_revision {
            *rev
        } else if let Some(head) = self
            .resolve_head_revision(tenant_id, repo_id, branch)
            .await?
        {
            head
        } else {
            return Ok(None);
        };

        self.get_at_revision_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            id,
            &target_revision,
            true,
        )
        .await
    }

    /// Dispatch for `NodeRepository::get_with_children`.
    pub(crate) async fn dispatch_get_with_children(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        id: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<NodeWithPopulatedChildren>> {
        let node = match self
            .get(
                StorageScope::new(tenant_id, repo_id, branch, workspace),
                id,
                max_revision,
            )
            .await?
        {
            Some(n) => n,
            None => return Ok(None),
        };

        let children = self
            .list_children(
                StorageScope::new(tenant_id, repo_id, branch, workspace),
                &node.path,
                ListOptions {
                    compute_has_children: false,
                    max_revision: max_revision.cloned(),
                },
            )
            .await?;

        Ok(Some(NodeWithPopulatedChildren {
            node,
            children_nodes: children,
        }))
    }

    /// Dispatch for `NodeRepository::create`.
    pub(crate) async fn dispatch_create(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node: Node,
        options: CreateNodeOptions,
    ) -> Result<()> {
        self.validate_for_create(tenant_id, repo_id, branch, workspace, &node, &options)
            .await?;
        self.add_impl(tenant_id, repo_id, branch, workspace, node)
            .await
    }

    /// Dispatch for `NodeRepository::update`.
    pub(crate) async fn dispatch_update(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node: Node,
        options: UpdateNodeOptions,
    ) -> Result<()> {
        self.validate_for_update(tenant_id, repo_id, branch, workspace, &node, &options)
            .await?;
        self.update_impl(tenant_id, repo_id, branch, workspace, node)
            .await
    }

    /// Dispatch for `NodeRepository::delete`.
    pub(crate) async fn dispatch_delete(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        id: &str,
        options: DeleteNodeOptions,
    ) -> Result<bool> {
        if options.cascade {
            self.delete_with_cascade(tenant_id, repo_id, branch, workspace, id)
                .await
        } else {
            self.delete_without_cascade(
                tenant_id,
                repo_id,
                branch,
                workspace,
                id,
                options.check_has_children,
            )
            .await
        }
    }
}
