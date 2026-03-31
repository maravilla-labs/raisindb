// SPDX-License-Identifier: BSL-1.1
//! Relation operations: add-relation, remove-relation.

use raisin_storage::Storage;

use crate::error::ApiError;

use super::common::{CommandContext, CommandResult};

/// Handle the add-relation command.
pub async fn handle_add_relation<S: Storage>(ctx: &mut CommandContext<'_, S>) -> CommandResult {
    let target_workspace = ctx.params.target_workspace.clone().ok_or_else(|| {
        ApiError::validation_failed("target_workspace is required for add-relation")
    })?;
    let target_path = ctx.params.target_path.clone().ok_or_else(|| {
        ApiError::validation_failed("target_path is required for add-relation")
    })?;
    let weight = ctx.params.weight;
    let relation_type = ctx.params.relation_type.clone();

    ctx.nodes_svc
        .add_relation(ctx.path, &target_workspace, &target_path, weight, relation_type)
        .await?;

    CommandContext::<S>::ok_empty()
}

/// Handle the remove-relation command.
pub async fn handle_remove_relation<S: Storage>(ctx: &mut CommandContext<'_, S>) -> CommandResult {
    let target_workspace = ctx.params.target_workspace.clone().ok_or_else(|| {
        ApiError::validation_failed("target_workspace is required for remove-relation")
    })?;
    let target_path = ctx.params.target_path.clone().ok_or_else(|| {
        ApiError::validation_failed("target_path is required for remove-relation")
    })?;

    let removed = ctx
        .nodes_svc
        .remove_relation(ctx.path, &target_workspace, &target_path)
        .await?;

    CommandContext::<S>::ok_json(serde_json::json!({ "removed": removed }))
}
