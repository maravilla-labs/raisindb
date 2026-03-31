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

//! Listing / query dispatch helpers for `NodeRepository`.
//!
//! Methods that call an internal `_impl` and optionally compute
//! `has_children` for the result set.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_storage::ListOptions;

use crate::repositories::nodes::NodeRepositoryImpl;

impl NodeRepositoryImpl {
    /// Dispatch for `NodeRepository::list_by_type`.
    pub(crate) async fn dispatch_list_by_type(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_type: &str,
        options: ListOptions,
    ) -> Result<Vec<Node>> {
        let mut nodes = self
            .list_by_type_impl(
                tenant_id,
                repo_id,
                branch,
                workspace,
                node_type,
                options.max_revision.as_ref(),
            )
            .await?;

        if options.compute_has_children {
            self.populate_has_children(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &mut nodes,
                options.max_revision.as_ref(),
            )
            .await?;
        }

        Ok(nodes)
    }

    /// Dispatch for `NodeRepository::list_all`.
    pub(crate) async fn dispatch_list_all(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        options: ListOptions,
    ) -> Result<Vec<Node>> {
        let mut nodes = self
            .list_all_impl(
                tenant_id,
                repo_id,
                branch,
                workspace,
                options.max_revision.as_ref(),
            )
            .await?;

        if options.compute_has_children {
            self.populate_has_children(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &mut nodes,
                options.max_revision.as_ref(),
            )
            .await?;
        }

        Ok(nodes)
    }

    /// Dispatch for `NodeRepository::list_children`.
    pub(crate) async fn dispatch_list_children(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_path: &str,
        options: ListOptions,
    ) -> Result<Vec<Node>> {
        let mut nodes = self
            .list_children_impl(
                tenant_id,
                repo_id,
                branch,
                workspace,
                parent_path,
                options.max_revision.as_ref(),
            )
            .await?;

        if options.compute_has_children {
            self.populate_has_children(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &mut nodes,
                options.max_revision.as_ref(),
            )
            .await?;
        }

        Ok(nodes)
    }

    /// Dispatch for `NodeRepository::scan_descendants_ordered`.
    pub(crate) async fn dispatch_scan_descendants_ordered(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_node_id: &str,
        options: ListOptions,
    ) -> Result<Vec<Node>> {
        let nodes_with_depth = self.scan_descendants_ordered_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_node_id,
            options.max_revision.as_ref(),
        )?;

        Ok(nodes_with_depth
            .into_iter()
            .map(|(node, _depth)| node)
            .collect())
    }
}
