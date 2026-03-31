// SPDX-License-Identifier: BSL-1.1
//! Versioning operations: create_version, restore_version, delete_version, update_version_note.

use axum::http::StatusCode;
use axum::Json;
use raisin_audit::AuditRepository;
use raisin_storage::Storage;

use crate::error::ApiError;

use super::common::{CommandContext, CommandResult};

/// Handle the create_version command.
pub async fn handle_create_version<S: Storage>(ctx: &mut CommandContext<'_, S>) -> CommandResult {
    let note = ctx.params.note.clone();
    let version_num = ctx.nodes_svc.create_manual_version(ctx.path, note).await?;
    CommandContext::<S>::ok_json(serde_json::json!({"version": version_num}))
}

/// Handle the restore_version command.
pub async fn handle_restore_version<S: Storage>(ctx: &mut CommandContext<'_, S>) -> CommandResult {
    let version = ctx.params.version.ok_or_else(|| {
        ApiError::validation_failed("version is required for restore_version command")
    })?;
    let restored_node = ctx.nodes_svc.restore_version(ctx.path, version).await?;
    Ok((
        StatusCode::OK,
        Json(serde_json::to_value(restored_node).unwrap_or_default()),
    ))
}

/// Handle the delete_version command.
pub async fn handle_delete_version<S: Storage>(ctx: &mut CommandContext<'_, S>) -> CommandResult {
    let version = ctx.params.version.ok_or_else(|| {
        ApiError::validation_failed("version is required for delete_version command")
    })?;
    let deleted = ctx.nodes_svc.delete_version(ctx.path, version).await?;
    CommandContext::<S>::ok_json(serde_json::json!({"deleted": deleted}))
}

/// Handle the update_version_note command.
pub async fn handle_update_version_note<S: Storage>(
    ctx: &mut CommandContext<'_, S>,
) -> CommandResult {
    let version = ctx.params.version.ok_or_else(|| {
        ApiError::validation_failed("version is required for update_version_note command")
    })?;
    let note = ctx.params.note.clone();
    ctx.nodes_svc
        .update_version_note(ctx.path, version, note)
        .await?;
    CommandContext::<S>::ok_empty()
}

/// Handle the audit_log command.
pub async fn handle_audit_log<S: Storage>(ctx: &mut CommandContext<'_, S>) -> CommandResult {
    // fetch by node path
    let node = ctx
        .nodes_svc
        .get_by_path(ctx.path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(ctx.path))?;
    let logs = ctx.state.audit.get_logs_by_node_id(&node.id).await?;
    Ok((
        StatusCode::OK,
        Json(serde_json::to_value(logs).expect("audit logs should serialize to JSON")),
    ))
}
