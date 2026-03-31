//! User and workspace operation handlers for replication
//!
//! This module contains operation handlers for:
//! - apply_update_user
//! - apply_delete_user
//! - apply_update_workspace
//! - apply_delete_workspace

use super::super::OperationApplicator;
use super::db_helpers::{delete_key, serialize_and_write_compact};
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_models::admin_user::DatabaseAdminUser;
use raisin_models::workspace::Workspace;
use raisin_replication::Operation;

/// Apply a user update operation
pub(super) async fn apply_update_user(
    applicator: &OperationApplicator,
    tenant_id: &str,
    user_id: &str,
    user: &DatabaseAdminUser,
    op: &Operation,
) -> Result<()> {
    tracing::info!(
        "📥 Applying user update: {}/{} from node {}",
        tenant_id,
        user_id,
        op.cluster_node_id
    );

    // Use username instead of user_id to match AdminUserStore.build_key() format
    let key = keys::admin_user_key(tenant_id, &user.username);
    let cf = cf_handle(&applicator.db, cf::ADMIN_USERS)?;

    let value = rmp_serde::to_vec(&user)
        .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

    applicator
        .db
        .put_cf(cf, key, value)
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

    tracing::info!("✅ User applied successfully: {}/{}", tenant_id, user_id);
    Ok(())
}

/// Apply a user delete operation
pub(super) async fn apply_delete_user(
    applicator: &OperationApplicator,
    tenant_id: &str,
    user_id: &str,
    op: &Operation,
) -> Result<()> {
    tracing::info!(
        "📥 Applying user delete: {}/{} from node {}",
        tenant_id,
        user_id,
        op.cluster_node_id
    );

    // Note: user_id parameter is actually the username (passed from capture_delete_user)
    let key = keys::admin_user_key(tenant_id, user_id);
    let cf = cf_handle(&applicator.db, cf::ADMIN_USERS)?;

    applicator
        .db
        .delete_cf(cf, key)
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

    tracing::info!("✅ User deleted successfully: {}/{}", tenant_id, user_id);
    Ok(())
}

/// Apply a workspace update operation
pub(super) async fn apply_update_workspace(
    applicator: &OperationApplicator,
    tenant_id: &str,
    repo_id: &str,
    workspace_id: &str,
    workspace: &Workspace,
    op: &Operation,
) -> Result<()> {
    tracing::info!(
        "📥 Applying workspace update: {}/{}/{} from node {}",
        tenant_id,
        repo_id,
        workspace_id,
        op.cluster_node_id
    );

    let key = keys::workspace_key(tenant_id, repo_id, workspace_id);
    let cf = cf_handle(&applicator.db, cf::WORKSPACES)?;

    serialize_and_write_compact(
        &applicator.db,
        cf,
        key,
        workspace,
        &format!(
            "apply_update_workspace_{}/{}/{}",
            tenant_id, repo_id, workspace_id
        ),
    )?;

    tracing::info!(
        "✅ Workspace applied successfully: {}/{}/{}",
        tenant_id,
        repo_id,
        workspace_id
    );
    Ok(())
}

/// Apply a workspace delete operation
pub(super) async fn apply_delete_workspace(
    applicator: &OperationApplicator,
    tenant_id: &str,
    repo_id: &str,
    workspace_id: &str,
    op: &Operation,
) -> Result<()> {
    tracing::info!(
        "📥 Applying workspace delete: {}/{}/{} from node {}",
        tenant_id,
        repo_id,
        workspace_id,
        op.cluster_node_id
    );

    let key = keys::workspace_key(tenant_id, repo_id, workspace_id);
    let cf = cf_handle(&applicator.db, cf::WORKSPACES)?;

    delete_key(
        &applicator.db,
        cf,
        key,
        &format!(
            "apply_delete_workspace_{}/{}/{}",
            tenant_id, repo_id, workspace_id
        ),
    )?;

    tracing::info!(
        "✅ Workspace deleted successfully: {}/{}/{}",
        tenant_id,
        repo_id,
        workspace_id
    );
    Ok(())
}
