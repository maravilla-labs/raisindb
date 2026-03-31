// SPDX-License-Identifier: BSL-1.1
//! Node operations: rename, move, copy, copy_tree.

use axum::http::StatusCode;
use axum::Json;
use raisin_storage::Storage;

use crate::error::ApiError;

use super::common::{CommandContext, CommandResult};

/// Handle the rename command.
pub async fn handle_rename<S: Storage>(ctx: &mut CommandContext<'_, S>) -> CommandResult {
    let new_name = ctx.params.new_name.clone().ok_or_else(|| {
        ApiError::validation_failed("new_name is required for rename command")
    })?;

    tracing::info!(
        "Renaming '{}' to '{}' in workspace '{}'",
        ctx.path,
        new_name,
        ctx.ws
    );

    // Check if commit metadata is provided
    if let (Some(message), Some(actor)) = (&ctx.params.message, &ctx.params.actor) {
        // Transaction mode: use node_id for stability
        let node = ctx
            .nodes_svc
            .get_by_path(ctx.path)
            .await?
            .ok_or_else(|| ApiError::node_not_found(ctx.path))?;

        let mut tx = ctx.nodes_svc.transaction();
        tx.rename(node.id.clone(), new_name);
        let revision = tx.commit(message.clone(), actor.clone()).await?;

        return CommandContext::<S>::committed(revision);
    }

    // Direct mode (legacy): call storage directly
    ctx.nodes_svc.rename_node(ctx.path, &new_name).await?;
    CommandContext::<S>::ok_empty()
}

/// Handle the move command.
pub async fn handle_move<S: Storage>(ctx: &mut CommandContext<'_, S>) -> CommandResult {
    let new_path = ctx.params.target_path.clone().ok_or_else(|| {
        ApiError::validation_failed("target_path is required for move command")
    })?;

    tracing::info!(
        "Moving '{}' to '{}' in workspace '{}'",
        ctx.path,
        new_path,
        ctx.ws
    );

    // Fetch node id by current path
    let node = ctx
        .nodes_svc
        .get_by_path(ctx.path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(ctx.path))?;

    // Check if commit metadata is provided
    if let (Some(message), Some(actor)) = (&ctx.params.message, &ctx.params.actor) {
        // Transaction mode: use node_id for stability
        let mut tx = ctx.nodes_svc.transaction();
        tx.move_node(node.id.clone(), new_path.clone());
        let revision = tx.commit(message.clone(), actor.clone()).await?;

        return CommandContext::<S>::committed(revision);
    }

    // Direct mode (legacy): call storage directly
    ctx.nodes_svc.move_node(&node.id, &new_path).await?;
    CommandContext::<S>::ok_empty()
}

/// Handle the copy command.
pub async fn handle_copy<S: Storage>(ctx: &mut CommandContext<'_, S>) -> CommandResult {
    let target_path = ctx.params.target_path.clone().ok_or_else(|| {
        ApiError::validation_failed("target_path is required for copy command")
    })?;

    tracing::info!(
        "Copying '{}' to '{}' in workspace '{}'",
        ctx.path,
        target_path,
        ctx.ws
    );

    // Check if commit metadata is provided
    if let (Some(message), Some(actor)) = (&ctx.params.message, &ctx.params.actor) {
        // Transaction mode: use path-based copy
        // When newName is provided: targetPath is the parent, use newName
        // When newName is absent: targetPath is the parent, use source node's name
        let (target_parent, new_name) = if let Some(name) = &ctx.params.new_name {
            // Mode 1: targetPath is parent, newName is the desired name
            (target_path.clone(), Some(name.clone()))
        } else {
            // Mode 2: targetPath is parent, use source node's name
            (target_path.clone(), None)
        };

        let mut tx = ctx.nodes_svc.transaction();
        tx.copy(ctx.path.to_string(), target_parent, new_name);
        let revision = tx.commit(message.clone(), actor.clone()).await?;

        return CommandContext::<S>::committed(revision);
    }

    // Direct mode: delegate completely to service layer
    let copied = ctx
        .nodes_svc
        .copy_node_flexible(ctx.path, &target_path, ctx.params.new_name.as_deref())
        .await?;

    Ok((
        StatusCode::OK,
        Json(serde_json::to_value(copied).unwrap_or_default()),
    ))
}

/// Handle the copy_tree command.
pub async fn handle_copy_tree<S: Storage>(ctx: &mut CommandContext<'_, S>) -> CommandResult {
    let target_path = ctx.params.target_path.clone().ok_or_else(|| {
        ApiError::validation_failed("target_path is required for copy_tree command")
    })?;

    tracing::info!(
        "Copying tree '{}' to '{}' in workspace '{}'",
        ctx.path,
        target_path,
        ctx.ws
    );

    // Check if commit metadata is provided
    if let (Some(message), Some(actor)) = (&ctx.params.message, &ctx.params.actor) {
        // Transaction mode: use path-based recursive copy
        // When newName is provided: targetPath is the parent, use newName
        // When newName is absent: targetPath is the parent, use source node's name
        let (target_parent, new_name) = if let Some(name) = &ctx.params.new_name {
            // Mode 1: targetPath is parent, newName is the desired name
            (target_path.clone(), Some(name.clone()))
        } else {
            // Mode 2: targetPath is parent, use source node's name
            (target_path.clone(), None)
        };

        let mut tx = ctx.nodes_svc.transaction();
        tx.copy_tree(ctx.path.to_string(), target_parent, new_name);
        let revision = tx.commit(message.clone(), actor.clone()).await?;

        return CommandContext::<S>::committed(revision);
    }

    // Direct mode: delegate to service layer with flexible path handling
    let copied = ctx
        .nodes_svc
        .copy_node_tree_flexible(ctx.path, &target_path, ctx.params.new_name.as_deref())
        .await?;

    Ok((
        StatusCode::OK,
        Json(serde_json::to_value(copied).unwrap_or_default()),
    ))
}

/// Handle the reorder command.
pub async fn handle_reorder<S: Storage>(ctx: &mut CommandContext<'_, S>) -> CommandResult {
    let target_path = ctx.params.target_path.clone().ok_or_else(|| {
        ApiError::validation_failed("target_path is required for reorder command")
    })?;
    let move_position = ctx
        .params
        .move_position
        .clone()
        .unwrap_or_else(|| "after".into());
    let message = ctx.params.message.as_deref();
    let actor = ctx.params.actor.as_deref();

    // determine parent path and child names
    let current_node = ctx
        .nodes_svc
        .get_by_path(ctx.path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(ctx.path))?;
    let parent_path = current_node
        .parent_path()
        .unwrap_or_else(|| "/".to_string());
    let current_name = current_node.name.clone();
    // Derive target sibling name from path
    let target_name = target_path.rsplit('/').next().unwrap_or("");

    // For non-root parent, enforce that target exists under the same parent
    if parent_path != "/" && !parent_path.is_empty() {
        let siblings = ctx.nodes_svc.list_children(&parent_path).await?;
        if !siblings.iter().any(|n| n.name == target_name) {
            return Err(ApiError::node_not_found(&target_path));
        }
    }

    if move_position == "before" {
        ctx.nodes_svc
            .move_child_before(&parent_path, &current_name, target_name, message, actor)
            .await?;
    } else if move_position == "after" {
        ctx.nodes_svc
            .move_child_after(&parent_path, &current_name, target_name, message, actor)
            .await?;
    } else {
        return Err(ApiError::validation_failed(
            "move_position must be 'before' or 'after'",
        ));
    }

    CommandContext::<S>::ok_empty()
}
