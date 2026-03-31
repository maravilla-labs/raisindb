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

//! NodeRepository trait implementation for NodeRepositoryImpl.
//!
//! This is a pure dispatcher: every method delegates to an internal
//! helper defined in one of the submodules below.
//!
//! Submodules:
//! - `crud_dispatch`: get, create, update, delete dispatch helpers
//! - `listing_dispatch`: list_by_type, list_all, list_children, scan helpers
//! - `has_children_ext`: shared `populate_has_children` loop
//! - `workspace_validation`: parent-child and workspace type validation

mod crud_dispatch;
mod has_children_ext;
mod listing_dispatch;
mod workspace_validation;

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::{DeepNode, Node, NodeWithChildren};
use raisin_storage::{
    BranchScope, CreateNodeOptions, DeleteNodeOptions, ListOptions, NodeRepository,
    NodeWithPopulatedChildren, StorageScope, UpdateNodeOptions,
};
use std::collections::HashMap;

use super::NodeRepositoryImpl;

impl NodeRepository for NodeRepositoryImpl {
    // -- CRUD -----------------------------------------------------------------

    async fn get(
        &self,
        scope: StorageScope<'_>,
        id: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<Node>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.dispatch_get(tenant_id, repo_id, branch, workspace, id, max_revision)
            .await
    }

    async fn get_with_children(
        &self,
        scope: StorageScope<'_>,
        id: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<NodeWithPopulatedChildren>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.dispatch_get_with_children(tenant_id, repo_id, branch, workspace, id, max_revision)
            .await
    }

    async fn create(
        &self,
        scope: StorageScope<'_>,
        node: Node,
        options: CreateNodeOptions,
    ) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.dispatch_create(tenant_id, repo_id, branch, workspace, node, options)
            .await
    }

    async fn create_deep_node(
        &self,
        scope: StorageScope<'_>,
        path: &str,
        node: Node,
        parent_node_type: &str,
        options: CreateNodeOptions,
    ) -> Result<Node> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.create_deep_node_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            path,
            node,
            parent_node_type,
            options,
        )
        .await
    }

    async fn update(
        &self,
        scope: StorageScope<'_>,
        node: Node,
        options: UpdateNodeOptions,
    ) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.dispatch_update(tenant_id, repo_id, branch, workspace, node, options)
            .await
    }

    async fn delete(
        &self,
        scope: StorageScope<'_>,
        id: &str,
        options: DeleteNodeOptions,
    ) -> Result<bool> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.dispatch_delete(tenant_id, repo_id, branch, workspace, id, options)
            .await
    }

    async fn delete_by_path(
        &self,
        scope: StorageScope<'_>,
        path: &str,
        _options: DeleteNodeOptions,
    ) -> Result<bool> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.delete_by_path_impl(tenant_id, repo_id, branch, workspace, path)
            .await
    }

    // -- Listing / queries ----------------------------------------------------

    async fn list_by_type(
        &self,
        scope: StorageScope<'_>,
        node_type: &str,
        options: ListOptions,
    ) -> Result<Vec<Node>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.dispatch_list_by_type(tenant_id, repo_id, branch, workspace, node_type, options)
            .await
    }

    async fn list_by_parent(
        &self,
        scope: StorageScope<'_>,
        parent: &str,
        options: ListOptions,
    ) -> Result<Vec<Node>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.list_by_parent_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent,
            options.max_revision.as_ref(),
            options.compute_has_children,
        )
        .await
    }

    async fn get_by_path(
        &self,
        scope: StorageScope<'_>,
        path: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<Node>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.get_by_path_impl(tenant_id, repo_id, branch, workspace, path, max_revision)
            .await
    }

    async fn get_node_id_by_path(
        &self,
        scope: StorageScope<'_>,
        path: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<String>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.get_node_id_by_path_impl(tenant_id, repo_id, branch, workspace, path, max_revision)
            .await
    }

    async fn list_all(&self, scope: StorageScope<'_>, options: ListOptions) -> Result<Vec<Node>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.dispatch_list_all(tenant_id, repo_id, branch, workspace, options)
            .await
    }

    async fn count_all(
        &self,
        scope: StorageScope<'_>,
        max_revision: Option<&HLC>,
    ) -> Result<usize> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.count_all_impl(tenant_id, repo_id, branch, workspace, max_revision)
            .await
    }

    async fn scan_by_path_prefix(
        &self,
        scope: StorageScope<'_>,
        path_prefix: &str,
        options: ListOptions,
    ) -> Result<Vec<Node>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.scan_by_path_prefix_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            path_prefix,
            options.max_revision.as_ref(),
            options.compute_has_children,
        )
        .await
    }

    async fn scan_descendants_ordered(
        &self,
        scope: StorageScope<'_>,
        parent_node_id: &str,
        options: ListOptions,
    ) -> Result<Vec<Node>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.dispatch_scan_descendants_ordered(
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_node_id,
            options,
        )
        .await
    }

    async fn list_root(&self, scope: StorageScope<'_>, options: ListOptions) -> Result<Vec<Node>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.list_by_parent_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            "/",
            options.max_revision.as_ref(),
            options.compute_has_children,
        )
        .await
    }

    async fn list_children(
        &self,
        scope: StorageScope<'_>,
        parent_path: &str,
        options: ListOptions,
    ) -> Result<Vec<Node>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.dispatch_list_children(tenant_id, repo_id, branch, workspace, parent_path, options)
            .await
    }

    async fn stream_ordered_child_ids(
        &self,
        scope: StorageScope<'_>,
        parent_id: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<String>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.get_ordered_child_ids(
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_id,
            max_revision,
        )
        .await
    }

    async fn has_children(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        max_revision: Option<&HLC>,
    ) -> Result<bool> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.has_children_impl(tenant_id, repo_id, branch, workspace, node_id, max_revision)
            .await
    }

    // -- Tree operations ------------------------------------------------------

    async fn move_node(
        &self,
        scope: StorageScope<'_>,
        id: &str,
        new_path: &str,
        operation_meta: Option<raisin_models::operations::OperationMeta>,
    ) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.move_node_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            id,
            new_path,
            operation_meta,
        )
        .await
    }

    async fn move_node_tree(
        &self,
        scope: StorageScope<'_>,
        id: &str,
        new_path: &str,
        operation_meta: Option<raisin_models::operations::OperationMeta>,
    ) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.move_node_tree_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            id,
            new_path,
            operation_meta,
        )
        .await
    }

    async fn rename_node(
        &self,
        scope: StorageScope<'_>,
        old_path: &str,
        new_name: &str,
    ) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.rename_node_impl(tenant_id, repo_id, branch, workspace, old_path, new_name)
            .await
    }

    async fn deep_children_nested(
        &self,
        scope: StorageScope<'_>,
        parent_path: &str,
        max_depth: u32,
        max_revision: Option<&HLC>,
    ) -> Result<HashMap<String, DeepNode>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.deep_children_nested_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_path,
            max_depth,
            max_revision,
        )
        .await
    }

    async fn deep_children_flat(
        &self,
        scope: StorageScope<'_>,
        parent_path: &str,
        max_depth: u32,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<Node>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.deep_children_flat_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_path,
            max_depth,
            max_revision,
        )
        .await
    }

    async fn deep_children_array(
        &self,
        scope: StorageScope<'_>,
        parent_path: &str,
        max_depth: u32,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<NodeWithChildren>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.deep_children_array_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_path,
            max_depth,
            max_revision,
        )
        .await
    }

    async fn copy_node(
        &self,
        scope: StorageScope<'_>,
        source_path: &str,
        target_parent: &str,
        new_name: Option<&str>,
        operation_meta: Option<raisin_models::operations::OperationMeta>,
    ) -> Result<Node> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.copy_node_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            source_path,
            target_parent,
            new_name,
            operation_meta,
        )
        .await
    }

    async fn copy_node_tree(
        &self,
        scope: StorageScope<'_>,
        source_path: &str,
        target_parent: &str,
        new_name: Option<&str>,
        operation_meta: Option<raisin_models::operations::OperationMeta>,
    ) -> Result<Node> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.copy_node_tree_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            source_path,
            target_parent,
            new_name,
            operation_meta,
        )
        .await
    }

    // -- Ordering -------------------------------------------------------------

    async fn reorder_child(
        &self,
        scope: StorageScope<'_>,
        parent_path: &str,
        child_name: &str,
        new_position: usize,
        message: Option<&str>,
        actor: Option<&str>,
    ) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.reorder_child_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_path,
            child_name,
            new_position,
            message,
            actor,
        )
        .await
    }

    async fn move_child_before(
        &self,
        scope: StorageScope<'_>,
        parent_path: &str,
        child_name: &str,
        before_child_name: &str,
        message: Option<&str>,
        actor: Option<&str>,
    ) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.move_child_before_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_path,
            child_name,
            before_child_name,
            message,
            actor,
        )
        .await
    }

    async fn move_child_after(
        &self,
        scope: StorageScope<'_>,
        parent_path: &str,
        child_name: &str,
        after_child_name: &str,
        message: Option<&str>,
        actor: Option<&str>,
    ) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.move_child_after_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_path,
            child_name,
            after_child_name,
            message,
            actor,
        )
        .await
    }

    // -- Properties -----------------------------------------------------------

    async fn get_property_by_path(
        &self,
        scope: StorageScope<'_>,
        node_path: &str,
        property_path: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<PropertyValue>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.get_property_by_path_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_path,
            property_path,
            max_revision,
        )
        .await
    }

    async fn update_property_by_path(
        &self,
        scope: StorageScope<'_>,
        node_path: &str,
        property_path: &str,
        value: PropertyValue,
    ) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.update_property_by_path_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_path,
            property_path,
            value,
        )
        .await
    }

    // -- Publishing -----------------------------------------------------------

    async fn publish(&self, scope: StorageScope<'_>, node_path: &str) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.publish_impl(tenant_id, repo_id, branch, workspace, node_path)
            .await
    }

    async fn publish_tree(&self, scope: StorageScope<'_>, node_path: &str) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.publish_tree_impl(tenant_id, repo_id, branch, workspace, node_path)
            .await
    }

    async fn unpublish(&self, scope: StorageScope<'_>, node_path: &str) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.unpublish_impl(tenant_id, repo_id, branch, workspace, node_path)
            .await
    }

    async fn unpublish_tree(&self, scope: StorageScope<'_>, node_path: &str) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.unpublish_tree_impl(tenant_id, repo_id, branch, workspace, node_path)
            .await
    }

    async fn get_published(&self, scope: StorageScope<'_>, id: &str) -> Result<Option<Node>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.get_published_impl(tenant_id, repo_id, branch, workspace, id)
            .await
    }

    async fn get_published_by_path(
        &self,
        scope: StorageScope<'_>,
        path: &str,
    ) -> Result<Option<Node>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.get_published_by_path_impl(tenant_id, repo_id, branch, workspace, path)
            .await
    }

    async fn list_published_children(
        &self,
        scope: StorageScope<'_>,
        parent_path: &str,
    ) -> Result<Vec<Node>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.list_published_children_impl(tenant_id, repo_id, branch, workspace, parent_path)
            .await
    }

    async fn list_published_root(&self, scope: StorageScope<'_>) -> Result<Vec<Node>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.list_published_root_impl(tenant_id, repo_id, branch, workspace)
            .await
    }

    // -- Search / discovery ---------------------------------------------------

    async fn find_by_property(
        &self,
        scope: StorageScope<'_>,
        property_name: &str,
        property_value: &PropertyValue,
    ) -> Result<Vec<Node>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.find_by_property_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            property_name,
            property_value,
        )
        .await
    }

    async fn find_nodes_with_property(
        &self,
        scope: StorageScope<'_>,
        property_name: &str,
    ) -> Result<Vec<Node>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.find_nodes_with_property_impl(tenant_id, repo_id, branch, workspace, property_name)
            .await
    }

    async fn get_descendants_bulk(
        &self,
        scope: StorageScope<'_>,
        parent_path: &str,
        max_depth: u32,
        max_revision: Option<&HLC>,
    ) -> Result<HashMap<String, Node>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        self.get_descendants_bulk_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_path,
            max_depth,
            max_revision,
        )
        .await
    }

    // -- Validation -----------------------------------------------------------

    async fn validate_parent_allows_child(
        &self,
        scope: BranchScope<'_>,
        parent_node_type: &str,
        child_node_type: &str,
    ) -> Result<()> {
        let BranchScope {
            tenant_id,
            repo_id,
            branch,
        } = scope;
        self.validate_parent_allows_child_impl(
            tenant_id,
            repo_id,
            branch,
            parent_node_type,
            child_node_type,
        )
        .await
    }

    async fn validate_workspace_allows_node_type(
        &self,
        scope: StorageScope<'_>,
        node_type: &str,
        is_root_node: bool,
    ) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            workspace,
            ..
        } = scope;
        self.validate_workspace_allows_node_type_impl(
            tenant_id,
            repo_id,
            workspace,
            node_type,
            is_root_node,
        )
        .await
    }
}
