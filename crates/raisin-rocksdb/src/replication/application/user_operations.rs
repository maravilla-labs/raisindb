//! User and workspace operation handlers for replication
//!
//! This module contains operation handlers for:
//! - apply_update_user
//! - apply_delete_user
//! - apply_update_workspace
//! - apply_delete_workspace

use super::super::OperationApplicator;
use super::db_helpers::{delete_key, serialize_and_write_compact};
use crate::admin_user_store::AdminUserStore;
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

    // Through the store, not a second copy of its key layout and encoding.
    // These used to be mirrored here with a comment asking the reader to keep
    // them in sync by hand; they agreed, but nothing enforced it, and a replica
    // that keyed or encoded users even slightly differently would silently fail
    // to find users it holds. Constructed without capture, so applying cannot
    // re-emit.
    AdminUserStore::new(applicator.db().clone()).put_replicated(user)?;

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

    // `user_id` here is actually the username — that is what `capture_delete_user`
    // sends, and what the store keys by.
    AdminUserStore::new(applicator.db().clone()).delete_replicated(tenant_id, user_id)?;

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
