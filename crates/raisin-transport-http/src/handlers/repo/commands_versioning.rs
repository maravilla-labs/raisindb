// SPDX-License-Identifier: BSL-1.1

//! Versioning and audit log command handlers.
//!
//! Handles `restore_version`, `delete_version`, `update_version_note`,
//! and `audit_log` commands.

use axum::{extract::Json, http::StatusCode};
use raisin_core::NodeService;
use raisin_storage::{transactional::TransactionalStorage, Storage};

use crate::{error::ApiError, state::AppState, types::CommandBody};

/// Handle the `restore_version` command.
pub(super) async fn handle_restore_version<S: Storage + TransactionalStorage + 'static>(
    nodes_svc: &NodeService<S>,
    path: &str,
    params: &CommandBody,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let version = params.version.ok_or_else(|| {
        ApiError::validation_failed("version is required for restore_version command")
    })?;
    let restored_node = nodes_svc.restore_version(path, version).await?;
    Ok((
        StatusCode::OK,
        Json(serde_json::to_value(restored_node).unwrap_or_default()),
    ))
}

/// Handle the `delete_version` command.
pub(super) async fn handle_delete_version<S: Storage + TransactionalStorage + 'static>(
    nodes_svc: &NodeService<S>,
    path: &str,
    params: &CommandBody,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let version = params.version.ok_or_else(|| {
        ApiError::validation_failed("version is required for delete_version command")
    })?;
    let deleted = nodes_svc.delete_version(path, version).await?;
    Ok((
        StatusCode::OK,
        Json(serde_json::json!({"deleted": deleted})),
    ))
}

/// Handle the `update_version_note` command.
pub(super) async fn handle_update_version_note<S: Storage + TransactionalStorage + 'static>(
    nodes_svc: &NodeService<S>,
    path: &str,
    params: &CommandBody,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let version = params.version.ok_or_else(|| {
        ApiError::validation_failed("version is required for update_version_note command")
    })?;
    let note = params.note.clone();
    nodes_svc.update_version_note(path, version, note).await?;
    Ok((StatusCode::OK, Json(serde_json::json!({}))))
}

/// Handle the `audit_log` command: return audit log entries for a node.
#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_audit_log<S: Storage + TransactionalStorage + 'static>(
    state: &AppState,
    nodes_svc: &NodeService<S>,
    tenant_id: &str,
    repository: &str,
    branch: &str,
    workspace: &str,
    path: &str,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    use raisin_audit::{AuditRepository, AuditScope};

    let node = nodes_svc
        .get_by_path(path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(path))?;
    // MUST be the scoped read. The persistent store keys entries by
    // tenant/repo/branch/workspace and keeps no global node-id index, so the
    // unscoped call returns nothing at all against RocksDB — this command
    // silently returned `[]` for every node until it was scoped.
    let scope = AuditScope::new(tenant_id, repository, branch, workspace);
    let logs = state.audit.get_logs_scoped(scope, &node.id, None).await?;
    Ok((
        StatusCode::OK,
        Json(serde_json::to_value(logs).expect("audit logs should serialize to JSON")),
    ))
}
