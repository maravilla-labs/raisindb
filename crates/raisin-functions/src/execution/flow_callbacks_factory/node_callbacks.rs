// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Node operation callbacks (loader, saver, creator) for flow execution

use super::types::{ChildrenListerCallback, NodeCreatorCallback, NodeLoaderCallback, NodeSaverCallback};
use crate::execution::ExecutionDependencies;
use raisin_binary::BinaryStorage;
use raisin_storage::{transactional::TransactionalStorage, Storage, StorageScope};
use std::sync::Arc;

/// Create node loader callback - loads nodes from storage
pub(super) fn create_node_loader<S, B>(
    deps: &Arc<ExecutionDependencies<S, B>>,
) -> NodeLoaderCallback
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    let deps = deps.clone();
    Arc::new(move |tenant_id, repo_id, branch, workspace, path| {
        let deps = deps.clone();
        Box::pin(async move {
            use raisin_storage::NodeRepository;

            tracing::debug!(
                tenant_id = %tenant_id,
                repo_id = %repo_id,
                branch = %branch,
                workspace = %workspace,
                path = %path,
                "Flow node_loader callback"
            );

            match deps
                .storage
                .nodes()
                .get_by_path(
                    StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                    &path,
                    None,
                )
                .await
            {
                Ok(Some(node)) => {
                    // Convert node to JSON
                    serde_json::to_value(node)
                        .map(Some)
                        .map_err(|e| format!("Failed to serialize node: {}", e))
                }
                Ok(None) => Ok(None),
                Err(e) => Err(format!("Failed to load node: {}", e)),
            }
        })
    })
}

/// Create node saver callback - updates existing nodes
pub(super) fn create_node_saver<S, B>(deps: &Arc<ExecutionDependencies<S, B>>) -> NodeSaverCallback
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    let deps = deps.clone();
    Arc::new(
        move |tenant_id, repo_id, branch, workspace, path, properties| {
            let deps = deps.clone();
            Box::pin(async move {
                use raisin_storage::{NodeRepository, UpdateNodeOptions};

                tracing::debug!(
                    tenant_id = %tenant_id,
                    repo_id = %repo_id,
                    branch = %branch,
                    workspace = %workspace,
                    path = %path,
                    "Flow node_saver callback"
                );

                // First, load the existing node to get its ID and other fields
                let mut existing = deps
                    .storage
                    .nodes()
                    .get_by_path(
                        StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                        &path,
                        None,
                    )
                    .await
                    .map_err(|e| format!("Failed to load existing node: {}", e))?
                    .ok_or_else(|| format!("Node not found at path: {}", path))?;

                // Parse properties as a map
                let props_map: std::collections::HashMap<
                    String,
                    raisin_models::nodes::properties::PropertyValue,
                > = serde_json::from_value(properties)
                    .map_err(|e| format!("Failed to parse properties: {}", e))?;

                // Update the node's properties
                existing.properties = props_map;
                existing.updated_at = Some(chrono::Utc::now());

                // Update the node using the correct API
                deps.storage
                    .nodes()
                    .update(
                        StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                        existing,
                        UpdateNodeOptions::default(),
                    )
                    .await
                    .map_err(|e| format!("Failed to update node: {}", e))?;

                Ok(())
            })
        },
    )
}

/// Create node creator callback - creates new nodes
pub(super) fn create_node_creator<S, B>(
    deps: &Arc<ExecutionDependencies<S, B>>,
) -> NodeCreatorCallback
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    let deps = deps.clone();
    Arc::new(
        move |tenant_id, repo_id, branch, workspace, node_type, path, properties| {
            let deps = deps.clone();
            Box::pin(async move {
                use raisin_storage::{CreateNodeOptions, NodeRepository};

                tracing::debug!(
                    tenant_id = %tenant_id,
                    repo_id = %repo_id,
                    branch = %branch,
                    workspace = %workspace,
                    node_type = %node_type,
                    path = %path,
                    "Flow node_creator callback"
                );

                // Parse properties as a map
                let props_map: std::collections::HashMap<
                    String,
                    raisin_models::nodes::properties::PropertyValue,
                > = serde_json::from_value(properties.clone())
                    .map_err(|e| format!("Failed to parse properties: {}", e))?;

                // Extract name from path (last segment)
                let name = path
                    .rsplit('/')
                    .next()
                    .ok_or_else(|| format!("Invalid path: {}", path))?
                    .to_string();

                // Extract parent path (everything before last segment)
                let parent_path = if path.contains('/') {
                    path.rsplit_once('/')
                        .map(|(p, _)| if p.is_empty() { "/" } else { p })
                } else {
                    None
                };

                // Build a Node object
                let node_id = nanoid::nanoid!();
                let node = raisin_models::nodes::Node {
                    id: node_id.clone(),
                    name: name.clone(),
                    node_type: node_type.clone(),
                    path: path.to_string(),
                    properties: props_map,
                    parent: parent_path.map(String::from),
                    workspace: Some(workspace.to_string()),
                    created_at: Some(chrono::Utc::now()),
                    updated_at: Some(chrono::Utc::now()),
                    relations: vec![],
                    ..Default::default()
                };

                // Create the node using deep create to auto-create parent folders
                let created = deps.storage
                    .nodes()
                    .create_deep_node(
                        StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                        &path,
                        node,
                        "raisin:Folder",
                        CreateNodeOptions {
                            validate_schema: false,
                            validate_parent_allows_child: false,
                            validate_workspace_allows_type: false,
                            operation_meta: None,
                        },
                    )
                    .await
                    .map_err(|e| format!("Failed to create node: {}", e))?;

                serde_json::to_value(created)
                    .map_err(|e| format!("Failed to serialize created node: {}", e))
            })
        },
    )
}

/// Create children lister callback - lists child nodes under a parent path
pub(super) fn create_children_lister<S, B>(
    deps: &Arc<ExecutionDependencies<S, B>>,
) -> ChildrenListerCallback
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    let deps = deps.clone();
    Arc::new(move |tenant_id, repo_id, branch, workspace, parent_path| {
        let deps = deps.clone();
        Box::pin(async move {
            use raisin_storage::{ListOptions, NodeRepository};

            tracing::debug!(
                tenant_id = %tenant_id,
                repo_id = %repo_id,
                branch = %branch,
                workspace = %workspace,
                parent_path = %parent_path,
                "Flow children_lister callback"
            );

            let children = deps
                .storage
                .nodes()
                .list_children(
                    StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                    &parent_path,
                    ListOptions::default(),
                )
                .await
                .map_err(|e| format!("Failed to list children: {}", e))?;

            children
                .into_iter()
                .map(|node| {
                    serde_json::to_value(node)
                        .map_err(|e| format!("Failed to serialize child node: {}", e))
                })
                .collect()
        })
    })
}
