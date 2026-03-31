//! Workspace update application logic.

use chrono::Utc;
use raisin_models::workspace::Workspace;
use raisin_storage::system_updates::{AppliedDefinition, PendingUpdate, ResourceType};
use raisin_storage::{scope::RepoScope, Storage, SystemUpdateRepository, WorkspaceRepository};

use crate::error::ApiError;
use crate::state::AppState;

/// Apply a single Workspace system update.
///
/// Looks up the workspace from the provided global definitions, preserves the
/// existing created_at timestamp, then writes the updated definition to storage
/// and records the applied hash.
pub(super) async fn apply_workspace_update(
    state: &AppState,
    system_update_repo: &impl SystemUpdateRepository,
    tenant_id: &str,
    repo_id: &str,
    update: &PendingUpdate,
    workspaces: &[(Workspace, String)],
) -> Result<bool, ApiError> {
    let Some((mut workspace, hash)) = workspaces
        .iter()
        .find(|(ws, _)| ws.name == update.name)
        .cloned()
    else {
        return Ok(false);
    };

    // Set timestamps
    workspace.updated_at = Some(raisin_models::StorageTimestamp::now());

    // Check if it exists and preserve created_at
    if let Some(existing) = state
        .storage()
        .workspaces()
        .get(RepoScope::new(tenant_id, repo_id), &workspace.name)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to get existing Workspace: {}", e)))?
    {
        workspace.created_at = existing.created_at;
    }

    // Apply the update
    state
        .storage()
        .workspaces()
        .put(RepoScope::new(tenant_id, repo_id), workspace.clone())
        .await
        .map_err(|e| ApiError::internal(format!("Failed to apply Workspace update: {}", e)))?;

    // Record the applied hash
    system_update_repo
        .set_applied(
            tenant_id,
            repo_id,
            ResourceType::Workspace,
            &workspace.name,
            AppliedDefinition {
                content_hash: hash,
                applied_version: None, // Workspaces don't have versions
                applied_at: Utc::now(),
                applied_by: "admin".to_string(),
            },
        )
        .await
        .map_err(|e| ApiError::internal(format!("Failed to record applied hash: {}", e)))?;

    tracing::info!(
        tenant_id = %tenant_id,
        repo_id = %repo_id,
        workspace = %update.name,
        "Applied Workspace system update"
    );

    Ok(true)
}
