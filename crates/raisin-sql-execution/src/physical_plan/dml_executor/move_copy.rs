// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! MOVE and COPY execution for relocating and duplicating node trees.
//!
//! MOVE relocates a node and all its descendants to a new parent.
//! COPY duplicates a node (or entire tree) to a new location.

use crate::physical_plan::executor::{ExecutionContext, Row, RowStream};
use futures::stream;
use raisin_core::NodeService;
use raisin_error::Error;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::{NodeRepository, Storage, StorageScope};

use super::order::resolve_node_reference;

/// Threshold for large tree move operations.
const MOVE_LARGE_TREE_THRESHOLD: usize = 5000;

/// Threshold for large tree copy operations.
const COPY_LARGE_TREE_THRESHOLD: usize = 5000;

/// Execute a physical MOVE operation.
///
/// Moves a node and all its descendants to a new parent location by:
/// 1. Resolving source and target_parent node references to paths
/// 2. Validating existence and checking for circular references
/// 3. Executing the move via storage layer
/// 4. Returning affected_rows count (1 + number of descendants moved)
pub async fn execute_move<
    'a,
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
    source: &'a raisin_sql::ast::order::NodeReference,
    target_parent: &'a raisin_sql::ast::order::NodeReference,
    workspace: &'a Option<String>,
    branch_override: &'a Option<String>,
    ctx: &'a ExecutionContext<S>,
) -> Result<RowStream, Error> {
    let workspace_id = workspace
        .as_ref()
        .map(|w| w.as_str())
        .unwrap_or(&ctx.workspace);

    let branch = branch_override
        .as_ref()
        .map(|b| b.as_str())
        .unwrap_or(&ctx.branch);

    // Step 1: Resolve node references to paths
    let source_path = resolve_node_reference(source, workspace_id, branch, ctx).await?;
    let target_parent_path =
        resolve_node_reference(target_parent, workspace_id, branch, ctx).await?;

    tracing::debug!(
        "MOVE: Resolved source={} target_parent={}",
        source_path,
        target_parent_path
    );

    // Step 2: Get source node to verify it exists and get its name
    let source_node = ctx
        .storage
        .nodes()
        .get_by_path(
            StorageScope::new(&ctx.tenant_id, &ctx.repo_id, branch, workspace_id),
            &source_path,
            None,
        )
        .await?
        .ok_or_else(|| {
            Error::NotFound(format!("Source node at path '{}' not found", source_path))
        })?;

    // Step 3: Verify target_parent exists
    let _target_parent_node = ctx
        .storage
        .nodes()
        .get_by_path(
            StorageScope::new(&ctx.tenant_id, &ctx.repo_id, branch, workspace_id),
            &target_parent_path,
            None,
        )
        .await?
        .ok_or_else(|| {
            Error::NotFound(format!(
                "Target parent node at path '{}' not found",
                target_parent_path
            ))
        })?;

    // Step 4: Check for circular reference
    if target_parent_path.starts_with(&format!("{}/", source_path)) {
        return Err(Error::Validation(format!(
            "MOVE: Cannot move '{}' into its own descendant '{}'. This would create a circular reference.",
            source_path, target_parent_path
        )));
    }

    if source_path == target_parent_path {
        return Err(Error::Validation(
            "MOVE: Cannot move a node into itself".to_string(),
        ));
    }

    // Step 5: Count descendants to estimate tree size
    let descendants = ctx
        .storage
        .nodes()
        .deep_children_flat(
            StorageScope::new(&ctx.tenant_id, &ctx.repo_id, branch, workspace_id),
            &source_path,
            u32::MAX,
            None,
        )
        .await?;

    let tree_size = 1 + descendants.len();

    if tree_size > MOVE_LARGE_TREE_THRESHOLD {
        tracing::warn!(
            "MOVE: Large tree operation ({} nodes). Source: '{}' -> Target parent: '{}'",
            tree_size,
            source_path,
            target_parent_path
        );
    }

    // Step 6: Compute new path
    let source_name = source_path
        .rsplit('/')
        .next()
        .ok_or_else(|| Error::Validation(format!("Invalid source path: {}", source_path)))?;

    let new_path = if target_parent_path == "/" {
        format!("/{}", source_name)
    } else {
        format!(
            "{}/{}",
            target_parent_path.trim_end_matches('/'),
            source_name
        )
    };

    tracing::debug!(
        "MOVE: Moving '{}' to new path '{}' (tree size: {})",
        source_path,
        new_path,
        tree_size
    );

    // Step 7: Execute the move operation
    let use_active_txn = {
        let tx_lock = ctx.transaction_context.read().await;
        tx_lock.is_some()
    };

    if use_active_txn {
        use raisin_storage::transactional::TransactionalContext;
        tracing::debug!("MOVE using active transaction context");
        let tx_lock = ctx.transaction_context.read().await;
        let txn_ctx = tx_lock.as_ref().ok_or_else(|| {
            Error::InvalidState("Transaction context lost during execution".to_string())
        })?;

        txn_ctx
            .move_node_tree(workspace_id, &source_node.id, &new_path)
            .await?;
    } else {
        tracing::debug!("MOVE using auto-commit mode");
        let txn_ctx = ctx.storage.begin_context().await?;

        txn_ctx.set_tenant_repo(&ctx.tenant_id, &ctx.repo_id)?;
        txn_ctx.set_branch(branch)?;
        txn_ctx.set_message(&format!("MOVE {} TO {}", source_path, target_parent_path))?;
        txn_ctx.set_actor("sql-move")?;

        let auth = ctx
            .auth_context
            .clone()
            .unwrap_or_else(AuthContext::anonymous);
        txn_ctx.set_auth_context(auth)?;

        txn_ctx
            .move_node_tree(workspace_id, &source_node.id, &new_path)
            .await?;

        txn_ctx.commit().await?;
    }

    tracing::info!(
        "MOVE: Moved '{}' to '{}' in workspace '{}' ({} nodes affected)",
        source_path,
        new_path,
        workspace_id,
        tree_size
    );

    let mut result_row = Row::new();
    result_row.insert(
        "affected_rows".to_string(),
        PropertyValue::Integer(tree_size as i64),
    );

    Ok(Box::pin(stream::once(async move { Ok(result_row) })))
}

/// Execute a physical COPY operation.
///
/// Copies a node (or entire tree with COPY TREE) to a new location.
/// Uses NodeService for the actual copy to ensure translations and
/// fractional index allocation are handled correctly.
pub async fn execute_copy<
    'a,
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
    source: &'a raisin_sql::ast::order::NodeReference,
    target_parent: &'a raisin_sql::ast::order::NodeReference,
    new_name: &'a Option<String>,
    recursive: bool,
    workspace: &'a Option<String>,
    branch_override: &'a Option<String>,
    ctx: &'a ExecutionContext<S>,
) -> Result<RowStream, Error> {
    let workspace_id = workspace
        .as_ref()
        .map(|w| w.as_str())
        .unwrap_or(&ctx.workspace);

    let branch = branch_override
        .as_ref()
        .map(|b| b.as_str())
        .unwrap_or(&ctx.branch);

    // Step 1: Resolve node references to paths
    let source_path = resolve_node_reference(source, workspace_id, branch, ctx).await?;
    let target_parent_path =
        resolve_node_reference(target_parent, workspace_id, branch, ctx).await?;

    tracing::debug!(
        "COPY: Resolved source={} target_parent={} recursive={}",
        source_path,
        target_parent_path,
        recursive
    );

    // Step 2: Normalize target parent path
    let target_parent_path = target_parent_path.trim_end_matches('/');
    let target_parent_path = if target_parent_path.is_empty() {
        "/".to_string()
    } else {
        target_parent_path.to_string()
    };

    // Step 3: Verify source exists
    let source_node = ctx
        .storage
        .nodes()
        .get_by_path(
            StorageScope::new(&ctx.tenant_id, &ctx.repo_id, branch, workspace_id),
            &source_path,
            None,
        )
        .await?
        .ok_or_else(|| {
            Error::NotFound(format!("Source node at path '{}' not found", source_path))
        })?;

    // Step 4: Verify target_parent exists
    let _target_parent_node = ctx
        .storage
        .nodes()
        .get_by_path(
            StorageScope::new(&ctx.tenant_id, &ctx.repo_id, branch, workspace_id),
            &target_parent_path,
            None,
        )
        .await?
        .ok_or_else(|| {
            Error::NotFound(format!(
                "Target parent node at path '{}' not found",
                target_parent_path
            ))
        })?;

    // Step 5: Determine copy name
    let copy_name = new_name.clone().unwrap_or_else(|| source_node.name.clone());

    // Step 6: Check for circular reference (COPY TREE only)
    if recursive && target_parent_path.starts_with(&format!("{}/", source_path)) {
        return Err(Error::Validation(format!(
            "COPY TREE: Cannot copy '{}' into its own descendant '{}'. This would create infinite recursion.",
            source_path, target_parent_path
        )));
    }

    if source_path == target_parent_path {
        return Err(Error::Validation(
            "COPY: Cannot copy a node into itself".to_string(),
        ));
    }

    // Step 7: Count descendants to estimate tree size
    let tree_size = if recursive {
        let descendants = ctx
            .storage
            .nodes()
            .deep_children_flat(
                StorageScope::new(&ctx.tenant_id, &ctx.repo_id, branch, workspace_id),
                &source_path,
                u32::MAX,
                None,
            )
            .await?;
        1 + descendants.len()
    } else {
        1
    };

    if tree_size > COPY_LARGE_TREE_THRESHOLD {
        tracing::warn!(
            "COPY: Large tree operation ({} nodes). Source: '{}' -> Target parent: '{}'",
            tree_size,
            source_path,
            target_parent_path
        );
    }

    let op_name = if recursive { "COPY TREE" } else { "COPY" };
    tracing::debug!(
        "{}: Copying '{}' to parent '{}' (tree size: {})",
        op_name,
        source_path,
        target_parent_path,
        tree_size
    );

    // Step 8: Execute the copy via NodeService (handles translations properly)
    tracing::debug!("{} via NodeService (same as REST API)", op_name);

    let mut nodes_svc = NodeService::new_with_context(
        ctx.storage.clone(),
        ctx.tenant_id.to_string(),
        ctx.repo_id.to_string(),
        branch.to_string(),
        workspace_id.to_string(),
    );

    if let Some(ref auth) = ctx.auth_context {
        nodes_svc = nodes_svc.with_auth(auth.clone());
    }

    let copied_node = if recursive {
        nodes_svc
            .copy_node_tree_flexible(&source_path, &target_parent_path, Some(copy_name.as_str()))
            .await?
    } else {
        nodes_svc
            .copy_node_flexible(&source_path, &target_parent_path, Some(copy_name.as_str()))
            .await?
    };

    tracing::info!(
        "{}: Copied '{}' to '{}' in workspace '{}' ({} nodes affected)",
        op_name,
        source_path,
        copied_node.path,
        workspace_id,
        tree_size
    );

    let mut result_row = Row::new();
    result_row.insert(
        "affected_rows".to_string(),
        PropertyValue::Integer(tree_size as i64),
    );
    result_row.insert(
        "copied_root_path".to_string(),
        PropertyValue::String(copied_node.path),
    );

    Ok(Box::pin(stream::once(async move { Ok(result_row) })))
}
