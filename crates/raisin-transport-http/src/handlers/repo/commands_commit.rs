// SPDX-License-Identifier: BSL-1.1

//! Transaction-based command handlers: commit, save, create, delete.
//!
//! These commands modify nodes through the commit/transaction pipeline
//! rather than individual node operations.

use axum::{extract::Json, http::StatusCode};
use raisin_core::NodeService;
use raisin_models::auth::AuthContext;
use raisin_storage::{transactional::TransactionalStorage, NodeRepository, Storage, StorageScope};

use crate::{error::ApiError, state::AppState, types::CommandBody};

/// Handle the `commit` command: apply multiple transaction operations atomically.
pub(super) async fn handle_commit<S: Storage + TransactionalStorage + 'static>(
    state: &AppState,
    _nodes_svc: &NodeService<S>,
    tenant_id: &str,
    repository: &str,
    branch: &str,
    ws: &str,
    params: &CommandBody,
    auth: Option<AuthContext>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let message = params
        .message
        .as_ref()
        .ok_or_else(|| ApiError::validation_failed("message is required for commit command"))?;
    let actor = params.actor.clone().unwrap_or_else(|| {
        auth.as_ref()
            .map(|ctx| ctx.actor_id())
            .unwrap_or_else(|| "system".to_string())
    });

    let operations = params
        .operations
        .as_ref()
        .ok_or_else(|| ApiError::validation_failed("operations are required for commit command"))?;
    if operations.is_empty() {
        return Err(ApiError::validation_failed("operations cannot be empty"));
    }

    let tx_operations: Vec<raisin_core::TxOperation> = operations
        .iter()
        .map(|op| serde_json::from_value(op.clone()))
        .collect::<Result<Vec<_>, _>>()?;

    let operation_count = tx_operations.len();

    let connection = state.connection();
    let tenant = connection.tenant(tenant_id);
    let repo = tenant.repository(repository);
    let workspace = repo.workspace(ws);

    let mut tx = workspace.nodes().branch(branch).transaction();

    for op in tx_operations {
        match op {
            raisin_core::TxOperation::Create { node } => tx.create(*node),
            raisin_core::TxOperation::Update {
                node_id,
                properties,
            } => tx.update(node_id, properties),
            raisin_core::TxOperation::Delete { node_id } => tx.delete(node_id),
            raisin_core::TxOperation::Move { node_id, new_path } => tx.move_node(node_id, new_path),
            raisin_core::TxOperation::Rename { node_id, new_name } => tx.rename(node_id, new_name),
            raisin_core::TxOperation::Copy {
                source_path,
                target_parent,
                new_name,
            } => tx.copy(source_path, target_parent, new_name),
            raisin_core::TxOperation::CopyTree {
                source_path,
                target_parent,
                new_name,
            } => tx.copy_tree(source_path, target_parent, new_name),
        };
    }

    let revision = tx.commit(message.clone(), actor).await?;

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "revision": revision,
            "operations_count": operation_count
        })),
    ))
}

/// Handle the `save` command: update a single node via transaction.
pub(super) async fn handle_save<S: Storage + TransactionalStorage + 'static>(
    state: &AppState,
    nodes_svc: &NodeService<S>,
    tenant_id: &str,
    repository: &str,
    branch: &str,
    ws: &str,
    path: &str,
    params: &CommandBody,
    auth: Option<AuthContext>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let message = params
        .message
        .as_ref()
        .ok_or_else(|| ApiError::validation_failed("message is required for save command"))?;
    let actor = params.actor.clone().unwrap_or_else(|| {
        auth.as_ref()
            .map(|ctx| ctx.actor_id())
            .unwrap_or_else(|| "system".to_string())
    });

    let node = nodes_svc
        .get_by_path(path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(path))?;

    let connection = state.connection();
    let tenant = connection.tenant(tenant_id);
    let repo = tenant.repository(repository);
    let workspace = repo.workspace(ws);
    let mut tx = workspace.nodes().branch(branch).transaction();

    if let Some(operations) = &params.operations {
        if let Some(first_op) = operations.first() {
            if let Ok(op) = serde_json::from_value::<raisin_core::TxOperation>(first_op.clone()) {
                match op {
                    raisin_core::TxOperation::Update { properties, .. } => {
                        tx.update(node.id.clone(), properties);
                    }
                    _ => {
                        return Err(ApiError::validation_failed(
                            "save command requires an update operation",
                        ))
                    }
                }
            } else {
                return Err(ApiError::invalid_json("Failed to parse operation"));
            }
        } else {
            return Err(ApiError::validation_failed(
                "save command requires at least one operation",
            ));
        }
    } else {
        return Err(ApiError::validation_failed(
            "operations are required for save command",
        ));
    }

    let revision = tx.commit(message.clone(), actor).await?;

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "revision": revision,
            "operations_count": 1
        })),
    ))
}

/// Handle the `create` command: create a node via transaction.
pub(super) async fn handle_create_cmd(
    state: &AppState,
    tenant_id: &str,
    repository: &str,
    branch: &str,
    ws: &str,
    params: &CommandBody,
    auth: Option<AuthContext>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let message = params
        .message
        .as_ref()
        .ok_or_else(|| ApiError::validation_failed("message is required for create command"))?;
    let actor = params.actor.clone().unwrap_or_else(|| {
        auth.as_ref()
            .map(|ctx| ctx.actor_id())
            .unwrap_or_else(|| "system".to_string())
    });

    let operations = params
        .operations
        .as_ref()
        .ok_or_else(|| ApiError::validation_failed("operations are required for create command"))?;
    if operations.is_empty() {
        return Err(ApiError::validation_failed("operations cannot be empty"));
    }

    let tx_op: raisin_core::TxOperation = serde_json::from_value(operations[0].clone())
        .map_err(|e| ApiError::invalid_json(e.to_string()))?;

    let node = match tx_op {
        raisin_core::TxOperation::Create { node } => *node,
        _ => {
            return Err(ApiError::validation_failed(
                "create command requires a create operation",
            ))
        }
    };

    let connection = state.connection();
    let tenant = connection.tenant(tenant_id);
    let repo = tenant.repository(repository);
    let workspace = repo.workspace(ws);
    let mut tx = workspace.nodes().branch(branch).transaction();

    tx.create(node);

    let revision = tx.commit(message.clone(), actor).await?;

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "revision": revision,
            "operations_count": 1
        })),
    ))
}

/// Handle the `delete` command: delete a node and its descendants via transaction.
pub(super) async fn handle_delete_cmd<S: Storage + TransactionalStorage + 'static>(
    state: &AppState,
    nodes_svc: &NodeService<S>,
    tenant_id: &str,
    repository: &str,
    branch: &str,
    ws: &str,
    path: &str,
    params: &CommandBody,
    auth: Option<AuthContext>,
    branch_head: Option<raisin_hlc::HLC>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let message = params
        .message
        .as_ref()
        .ok_or_else(|| ApiError::validation_failed("message is required for delete command"))?;
    let actor = params.actor.clone().unwrap_or_else(|| {
        auth.as_ref()
            .map(|ctx| ctx.actor_id())
            .unwrap_or_else(|| "system".to_string())
    });

    let node = nodes_svc
        .get_by_path(path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(path))?;

    let mut ids_to_delete = vec![node.id.clone()];
    let descendants = state
        .storage()
        .nodes()
        .deep_children_flat(
            StorageScope::new(tenant_id, repository, branch, ws),
            path,
            100,
            branch_head.as_ref(),
        )
        .await?;
    for desc_node in descendants {
        ids_to_delete.push(desc_node.id);
    }

    let connection = state.connection();
    let tenant = connection.tenant(tenant_id);
    let repo = tenant.repository(repository);
    let workspace = repo.workspace(ws);
    let mut tx = workspace.nodes().branch(branch).transaction();

    for id in ids_to_delete {
        tx.delete(id);
    }

    let revision = tx.commit(message.clone(), actor).await?;

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "revision": revision,
            "operations_count": 1
        })),
    ))
}
