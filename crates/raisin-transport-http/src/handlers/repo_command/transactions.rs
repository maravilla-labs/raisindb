// SPDX-License-Identifier: BSL-1.1
//! Transaction operations: commit, save, create, delete.

use raisin_storage::{NodeRepository, Storage};

use crate::error::ApiError;

use super::common::{CommandContext, CommandResult};

/// Handle the commit command.
pub async fn handle_commit<S: Storage>(ctx: &mut CommandContext<'_, S>) -> CommandResult {
    // Create transaction and commit (creates repository revision)
    let message = ctx.params.message.clone().ok_or_else(|| {
        ApiError::validation_failed("message is required for commit command")
    })?;
    let actor = ctx.get_actor();

    let operations = ctx.params.operations.clone().ok_or_else(|| {
        ApiError::validation_failed("operations are required for commit command")
    })?;
    if operations.is_empty() {
        return Err(ApiError::validation_failed("operations cannot be empty"));
    }

    // Parse operations into TxOperation enum
    let tx_operations: Vec<raisin_core::TxOperation> = operations
        .into_iter()
        .map(|op| serde_json::from_value(op))
        .collect::<Result<Vec<_>, _>>()?;

    let operation_count = tx_operations.len();

    // Create transaction via connection API
    let connection = ctx.state.connection();
    let tenant = connection.tenant(ctx.tenant_id);
    let repo = tenant.repository(ctx.repository);
    let workspace = repo.workspace(ctx.ws);

    let mut tx = workspace.nodes().branch(ctx.branch).transaction();

    // Add all operations to transaction
    for op in tx_operations {
        match op {
            raisin_core::TxOperation::Create { node } => {
                tx.create(*node);
            }
            raisin_core::TxOperation::Update {
                node_id,
                properties,
            } => {
                tx.update(node_id, properties);
            }
            raisin_core::TxOperation::Delete { node_id } => {
                tx.delete(node_id);
            }
            raisin_core::TxOperation::Move { node_id, new_path } => {
                tx.move_node(node_id, new_path);
            }
            raisin_core::TxOperation::Rename { node_id, new_name } => {
                tx.rename(node_id, new_name);
            }
            raisin_core::TxOperation::Copy {
                source_path,
                target_parent,
                new_name,
            } => {
                tx.copy(source_path, target_parent, new_name);
            }
            raisin_core::TxOperation::CopyTree {
                source_path,
                target_parent,
                new_name,
            } => {
                tx.copy_tree(source_path, target_parent, new_name);
            }
        }
    }

    // Commit transaction
    let revision = tx.commit(message, actor).await?;

    CommandContext::<S>::committed_with_count(revision, operation_count)
}

/// Handle the save command.
pub async fn handle_save<S: Storage>(ctx: &mut CommandContext<'_, S>) -> CommandResult {
    // GitHub-like single-node commit for update
    // POST /path/to/node/raisin:cmd/save
    // { "message": "...", "actor": "...", "properties": {...} }

    let message = ctx.params.message.clone().ok_or_else(|| {
        ApiError::validation_failed("message is required for save command")
    })?;
    let actor = ctx.get_actor();

    // Get the node to update
    let node = ctx
        .nodes_svc
        .get_by_path(ctx.path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(ctx.path))?;

    // Create transaction
    let connection = ctx.state.connection();
    let tenant = connection.tenant(ctx.tenant_id);
    let repo = tenant.repository(ctx.repository);
    let workspace = repo.workspace(ctx.ws);
    let mut tx = workspace.nodes().branch(ctx.branch).transaction();

    // Add update operation with properties from params
    if let Some(operations) = ctx.params.operations.clone() {
        // Parse properties from first operation if provided
        if let Some(first_op) = operations.first() {
            if let Ok(op) =
                serde_json::from_value::<raisin_core::TxOperation>(first_op.clone())
            {
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

    // Commit
    let revision = tx.commit(message, actor).await?;

    CommandContext::<S>::committed_with_count(revision, 1)
}

/// Handle the create command.
pub async fn handle_create<S: Storage>(ctx: &mut CommandContext<'_, S>) -> CommandResult {
    // GitHub-like single-node commit for create
    // POST /parent/path/raisin:cmd/create
    // { "message": "...", "actor": "...", "operations": [{"type": "create", "node": {...}}] }

    let message = ctx.params.message.clone().ok_or_else(|| {
        ApiError::validation_failed("message is required for create command")
    })?;
    let actor = ctx.get_actor();

    // Parse node from operations
    let operations = ctx.params.operations.clone().ok_or_else(|| {
        ApiError::validation_failed("operations are required for create command")
    })?;
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

    // Create transaction
    let connection = ctx.state.connection();
    let tenant = connection.tenant(ctx.tenant_id);
    let repo = tenant.repository(ctx.repository);
    let workspace = repo.workspace(ctx.ws);
    let mut tx = workspace.nodes().branch(ctx.branch).transaction();

    tx.create(node);

    // Commit
    let revision = tx.commit(message, actor).await?;

    CommandContext::<S>::committed_with_count(revision, 1)
}

/// Handle the delete command.
pub async fn handle_delete<S: Storage>(ctx: &mut CommandContext<'_, S>) -> CommandResult {
    // GitHub-like single-node commit for delete
    // POST /path/to/node/raisin:cmd/delete
    // { "message": "Remove obsolete content", "actor": "alice" }

    let message = ctx.params.message.clone().ok_or_else(|| {
        ApiError::validation_failed("message is required for delete command")
    })?;
    let actor = ctx.get_actor();

    // Get the node to delete
    let node = ctx
        .nodes_svc
        .get_by_path(ctx.path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(ctx.path))?;

    // Collect node + descendants for cascade delete commit
    let mut ids_to_delete = vec![node.id.clone()];
    let descendants = ctx
        .state
        .storage()
        .nodes()
        .deep_children_flat(
            ctx.tenant_id,
            ctx.repository,
            ctx.branch,
            ctx.ws,
            ctx.path,
            100,
            ctx.branch_head.as_ref(),
        )
        .await?;
    for desc_node in descendants {
        ids_to_delete.push(desc_node.id);
    }

    // Create transaction
    let connection = ctx.state.connection();
    let tenant = connection.tenant(ctx.tenant_id);
    let repo = tenant.repository(ctx.repository);
    let workspace = repo.workspace(ctx.ws);
    let mut tx = workspace.nodes().branch(ctx.branch).transaction();

    for id in ids_to_delete {
        tx.delete(id);
    }

    // Commit
    let revision = tx.commit(message, actor).await?;

    CommandContext::<S>::committed_with_count(revision, 1)
}
