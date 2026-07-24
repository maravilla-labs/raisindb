//! Workspace update application logic.
//!
//! The write itself lives in `raisin_core::system_updates::apply_workspace` so the
//! admin-triggered apply and the startup resync share one implementation. This
//! module only resolves the definition for the pending update and maps errors.

use raisin_models::workspace::Workspace;
use raisin_storage::system_updates::PendingUpdate;
use raisin_storage::SystemUpdateRepository;

use crate::error::ApiError;
use crate::state::AppState;

use super::map_storage_err;

/// Apply a single Workspace system update.
pub(super) async fn apply_workspace_update(
    state: &AppState,
    system_update_repo: &impl SystemUpdateRepository,
    tenant_id: &str,
    repo_id: &str,
    update: &PendingUpdate,
    workspaces: &[(Workspace, String)],
) -> Result<bool, ApiError> {
    let Some((workspace, hash)) = workspaces.iter().find(|(ws, _)| ws.name == update.name) else {
        return Ok(false);
    };

    raisin_core::system_updates::apply_workspace(
        state.storage().clone(),
        system_update_repo,
        tenant_id,
        repo_id,
        workspace,
        hash,
        "admin", // TODO: Get from auth context
    )
    .await
    .map_err(|e| map_storage_err("Failed to apply Workspace update", e))?;

    Ok(true)
}
