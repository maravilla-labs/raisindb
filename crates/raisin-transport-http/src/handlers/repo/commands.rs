// SPDX-License-Identifier: BSL-1.1

//! Command execution handler for repository node operations.
//!
//! Handles the `raisin:cmd` pattern for operations like rename, move, copy,
//! publish, reorder, versioning, audit, commit, and translation commands.
//!
//! Transaction-based commands (commit, save, create, delete) are in
//! [`super::commands_commit`]. Versioning and audit commands are in
//! [`super::commands_versioning`].

use axum::{extract::Json, http::StatusCode};
use raisin_core::NodeService;
use raisin_models::auth::AuthContext;
use raisin_storage::{transactional::TransactionalStorage, BranchRepository, Storage};

use crate::{error::ApiError, state::AppState, types::CommandBody};

/// Execute a command on a repository node.
///
/// This is the central dispatch for all `raisin:cmd/*` POST operations.
pub async fn repo_execute_command(
    state: &AppState,
    tenant_id: &str,
    repository: &str,
    branch: &str,
    ws: &str,
    path: &str,
    command: &str,
    params: CommandBody,
    auth: Option<AuthContext>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    tracing::info!(
        "COMMAND: {}, tenant={}, repo={}, branch={}, ws={}, path={}",
        command,
        tenant_id,
        repository,
        branch,
        ws,
        path
    );

    // Get branch HEAD revision and bound queries to it for snapshot isolation
    let mut nodes_svc =
        state.node_service_for_context(tenant_id, repository, branch, ws, auth.clone());
    let branch_head = state
        .storage()
        .branches()
        .get_branch(tenant_id, repository, branch)
        .await?
        .map(|info| info.head);
    if let Some(head) = branch_head {
        nodes_svc = nodes_svc.at_revision(head);
    }

    match command {
        "rename" => handle_rename(&nodes_svc, path, &params, ws).await,
        "move" => handle_move(&nodes_svc, path, &params, ws).await,
        "copy" => handle_copy(&nodes_svc, path, &params).await,
        "copy_tree" => handle_copy_tree(&nodes_svc, path, &params).await,
        "publish" => {
            nodes_svc.publish(path).await?;
            Ok((StatusCode::OK, Json(serde_json::json!({}))))
        }
        "publish_tree" => {
            nodes_svc.publish_tree(path).await?;
            Ok((StatusCode::OK, Json(serde_json::json!({}))))
        }
        "unpublish" => {
            nodes_svc.unpublish(path).await?;
            Ok((StatusCode::OK, Json(serde_json::json!({}))))
        }
        "unpublish_tree" => {
            nodes_svc.unpublish_tree(path).await?;
            Ok((StatusCode::OK, Json(serde_json::json!({}))))
        }
        "reorder" => handle_reorder(&nodes_svc, path, &params).await,
        "create_version" => {
            let note = params.note;
            let version_num = nodes_svc.create_manual_version(path, note).await?;
            Ok((
                StatusCode::OK,
                Json(serde_json::json!({"version": version_num})),
            ))
        }
        "restore_version" => {
            super::commands_versioning::handle_restore_version(&nodes_svc, path, &params).await
        }
        "delete_version" => {
            super::commands_versioning::handle_delete_version(&nodes_svc, path, &params).await
        }
        "update_version_note" => {
            super::commands_versioning::handle_update_version_note(&nodes_svc, path, &params).await
        }
        "audit_log" => super::commands_versioning::handle_audit_log(state, &nodes_svc, path).await,
        "commit" => {
            super::commands_commit::handle_commit(
                state, &nodes_svc, tenant_id, repository, branch, ws, &params, auth,
            )
            .await
        }
        "save" => {
            super::commands_commit::handle_save(
                state, &nodes_svc, tenant_id, repository, branch, ws, path, &params, auth,
            )
            .await
        }
        "create" => {
            super::commands_commit::handle_create_cmd(
                state, tenant_id, repository, branch, ws, &params, auth,
            )
            .await
        }
        "delete" => {
            super::commands_commit::handle_delete_cmd(
                state,
                &nodes_svc,
                tenant_id,
                repository,
                branch,
                ws,
                path,
                &params,
                auth,
                branch_head,
            )
            .await
        }
        "add-relation" => handle_add_relation(&nodes_svc, path, &params).await,
        "remove-relation" => handle_remove_relation(&nodes_svc, path, &params).await,
        "translate" => {
            super::commands_translation::handle_translate(
                state, &nodes_svc, tenant_id, repository, branch, ws, path, &params, auth,
            )
            .await
        }
        "delete-translation" => {
            super::commands_translation::handle_delete_translation(
                state, &nodes_svc, tenant_id, repository, branch, ws, path, &params, auth,
            )
            .await
        }
        "hide-in-locale" => {
            super::commands_translation::handle_hide_in_locale(
                state, &nodes_svc, tenant_id, repository, branch, ws, path, &params, auth,
            )
            .await
        }
        "unhide-in-locale" => {
            super::commands_translation::handle_unhide_in_locale(
                state, &nodes_svc, tenant_id, repository, branch, ws, path, &params, auth,
            )
            .await
        }
        "translation-staleness" => {
            super::commands_translation::handle_translation_staleness(
                state, &nodes_svc, tenant_id, repository, branch, ws, path, &params, auth,
            )
            .await
        }
        "acknowledge-staleness" => {
            super::commands_translation::handle_acknowledge_staleness(
                state, &nodes_svc, tenant_id, repository, branch, ws, path, &params, auth,
            )
            .await
        }
        _ => Err(ApiError::new(
            StatusCode::NOT_IMPLEMENTED,
            "COMMAND_NOT_IMPLEMENTED",
            format!("Unknown command: {}", command),
        )),
    }
}

/// Handle the `rename` command.
async fn handle_rename<S: Storage + TransactionalStorage + 'static>(
    nodes_svc: &NodeService<S>,
    path: &str,
    params: &CommandBody,
    _ws: &str,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let new_name = params
        .new_name
        .as_ref()
        .ok_or_else(|| ApiError::validation_failed("new_name is required for rename command"))?;

    if let (Some(message), Some(actor)) = (&params.message, &params.actor) {
        let node = nodes_svc
            .get_by_path(path)
            .await?
            .ok_or_else(|| ApiError::node_not_found(path))?;

        let mut tx = nodes_svc.transaction();
        tx.rename(node.id.clone(), new_name.clone());
        let revision = tx.commit(message.clone(), actor.clone()).await?;

        return Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "revision": revision,
                "committed": true
            })),
        ));
    }

    nodes_svc.rename_node(path, new_name).await?;
    Ok((StatusCode::OK, Json(serde_json::json!({}))))
}

/// Handle the `move` command.
async fn handle_move<S: Storage + TransactionalStorage + 'static>(
    nodes_svc: &NodeService<S>,
    path: &str,
    params: &CommandBody,
    _ws: &str,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let new_path = params
        .target_path
        .as_ref()
        .ok_or_else(|| ApiError::validation_failed("target_path is required for move command"))?;

    let node = nodes_svc
        .get_by_path(path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(path))?;

    if let (Some(message), Some(actor)) = (&params.message, &params.actor) {
        let mut tx = nodes_svc.transaction();
        tx.move_node(node.id.clone(), new_path.clone());
        let revision = tx.commit(message.clone(), actor.clone()).await?;

        return Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "revision": revision,
                "committed": true
            })),
        ));
    }

    nodes_svc.move_node(&node.id, new_path).await?;
    Ok((StatusCode::OK, Json(serde_json::json!({}))))
}

/// Handle the `copy` command: copy a single node.
async fn handle_copy<S: Storage + TransactionalStorage + 'static>(
    nodes_svc: &NodeService<S>,
    path: &str,
    params: &CommandBody,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let target_path = params
        .target_path
        .as_ref()
        .ok_or_else(|| ApiError::validation_failed("target_path is required for copy command"))?;

    if let (Some(message), Some(actor)) = (&params.message, &params.actor) {
        let (target_parent, new_name) = if let Some(name) = &params.new_name {
            (target_path.clone(), Some(name.clone()))
        } else {
            (target_path.clone(), None)
        };

        let mut tx = nodes_svc.transaction();
        tx.copy(path.to_string(), target_parent, new_name);
        let revision = tx.commit(message.clone(), actor.clone()).await?;

        return Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "revision": revision,
                "committed": true
            })),
        ));
    }

    let copied = nodes_svc
        .copy_node_flexible(path, target_path, params.new_name.as_deref())
        .await?;

    Ok((
        StatusCode::OK,
        Json(serde_json::to_value(copied).unwrap_or_default()),
    ))
}

/// Handle the `copy_tree` command: deep copy a node and all descendants.
async fn handle_copy_tree<S: Storage + TransactionalStorage + 'static>(
    nodes_svc: &NodeService<S>,
    path: &str,
    params: &CommandBody,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let target_path = params.target_path.as_ref().ok_or_else(|| {
        ApiError::validation_failed("target_path is required for copy_tree command")
    })?;

    if let (Some(message), Some(actor)) = (&params.message, &params.actor) {
        let (target_parent, new_name) = if let Some(name) = &params.new_name {
            (target_path.clone(), Some(name.clone()))
        } else {
            (target_path.clone(), None)
        };

        let mut tx = nodes_svc.transaction();
        tx.copy_tree(path.to_string(), target_parent, new_name);
        let revision = tx.commit(message.clone(), actor.clone()).await?;

        return Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "revision": revision,
                "committed": true
            })),
        ));
    }

    let copied = nodes_svc
        .copy_node_tree_flexible(path, target_path, params.new_name.as_deref())
        .await?;

    Ok((
        StatusCode::OK,
        Json(serde_json::to_value(copied).unwrap_or_default()),
    ))
}

/// Handle the `reorder` command: move a child before or after another sibling.
async fn handle_reorder<S: Storage + TransactionalStorage + 'static>(
    nodes_svc: &NodeService<S>,
    path: &str,
    params: &CommandBody,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let target_path = params.target_path.as_ref().ok_or_else(|| {
        ApiError::validation_failed("target_path is required for reorder command")
    })?;
    let move_position = params.move_position.as_deref().unwrap_or("after");
    let message = params.message.as_deref();
    let actor = params.actor.as_deref();

    let current_node = nodes_svc
        .get_by_path(path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(path))?;
    let parent_path = current_node
        .parent_path()
        .unwrap_or_else(|| "/".to_string());
    let current_name = current_node.name.clone();
    let target_name = target_path.rsplit('/').next().unwrap_or("");

    if parent_path != "/" && !parent_path.is_empty() {
        let siblings = nodes_svc.list_children(&parent_path).await?;
        if !siblings.iter().any(|n| n.name == target_name) {
            return Err(ApiError::node_not_found(target_path));
        }
    }

    if move_position == "before" {
        nodes_svc
            .move_child_before(&parent_path, &current_name, target_name, message, actor)
            .await?;
    } else if move_position == "after" {
        nodes_svc
            .move_child_after(&parent_path, &current_name, target_name, message, actor)
            .await?;
    } else {
        return Err(ApiError::validation_failed(
            "move_position must be 'before' or 'after'",
        ));
    }
    Ok((StatusCode::OK, Json(serde_json::json!({}))))
}

/// Handle the `add-relation` command.
async fn handle_add_relation<S: Storage + TransactionalStorage + 'static>(
    nodes_svc: &NodeService<S>,
    path: &str,
    params: &CommandBody,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let target_workspace = params.target_workspace.as_ref().ok_or_else(|| {
        ApiError::validation_failed("target_workspace is required for add-relation")
    })?;
    let target_path = params
        .target_path
        .as_ref()
        .ok_or_else(|| ApiError::validation_failed("target_path is required for add-relation"))?;

    nodes_svc
        .add_relation(
            path,
            target_workspace,
            target_path,
            params.weight,
            params.relation_type.clone(),
        )
        .await?;

    Ok((StatusCode::OK, Json(serde_json::json!({}))))
}

/// Handle the `remove-relation` command.
async fn handle_remove_relation<S: Storage + TransactionalStorage + 'static>(
    nodes_svc: &NodeService<S>,
    path: &str,
    params: &CommandBody,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let target_workspace = params.target_workspace.as_ref().ok_or_else(|| {
        ApiError::validation_failed("target_workspace is required for remove-relation")
    })?;
    let target_path = params.target_path.as_ref().ok_or_else(|| {
        ApiError::validation_failed("target_path is required for remove-relation")
    })?;

    let removed = nodes_svc
        .remove_relation(path, target_workspace, target_path)
        .await?;

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({ "removed": removed })),
    ))
}
