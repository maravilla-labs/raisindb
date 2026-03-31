//! NodeType update application logic.

use chrono::Utc;
use raisin_models::nodes::NodeType;
use raisin_storage::system_updates::{AppliedDefinition, PendingUpdate, ResourceType};
use raisin_storage::{
    scope::BranchScope, CommitMetadata, NodeTypeRepository, Storage, SystemUpdateRepository,
};

use crate::error::ApiError;
use crate::state::AppState;

/// Apply a single NodeType system update.
///
/// Looks up the nodetype from the provided global definitions, preserves any
/// existing ID and created_at timestamp, then writes the updated definition
/// to storage and records the applied hash.
pub(super) async fn apply_nodetype_update(
    state: &AppState,
    system_update_repo: &impl SystemUpdateRepository,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    update: &PendingUpdate,
    nodetypes: &[(NodeType, String)],
) -> Result<bool, ApiError> {
    let Some((mut nodetype, hash)) = nodetypes
        .iter()
        .find(|(nt, _)| nt.name == update.name)
        .cloned()
    else {
        return Ok(false);
    };

    // Generate ID if not present
    if nodetype.id.is_none() {
        nodetype.id = Some(nanoid::nanoid!());
    }

    // Check if it exists and preserve ID
    if let Some(existing) = state
        .storage()
        .node_types()
        .get(
            BranchScope::new(tenant_id, repo_id, branch),
            &nodetype.name,
            None,
        )
        .await
        .map_err(|e| ApiError::internal(format!("Failed to get existing NodeType: {}", e)))?
    {
        nodetype.id = existing.id;
        nodetype.created_at = existing.created_at;
    }

    nodetype.updated_at = Some(Utc::now());

    // Apply the update
    state
        .storage()
        .node_types()
        .put(
            BranchScope::new(tenant_id, repo_id, branch),
            nodetype.clone(),
            CommitMetadata::system(format!("System update: {}", nodetype.name)),
        )
        .await
        .map_err(|e| ApiError::internal(format!("Failed to apply NodeType update: {}", e)))?;

    // Record the applied hash
    system_update_repo
        .set_applied(
            tenant_id,
            repo_id,
            ResourceType::NodeType,
            &nodetype.name,
            AppliedDefinition {
                content_hash: hash,
                applied_version: nodetype.version,
                applied_at: Utc::now(),
                applied_by: "admin".to_string(), // TODO: Get from auth context
            },
        )
        .await
        .map_err(|e| ApiError::internal(format!("Failed to record applied hash: {}", e)))?;

    tracing::info!(
        tenant_id = %tenant_id,
        repo_id = %repo_id,
        nodetype = %update.name,
        "Applied NodeType system update"
    );

    Ok(true)
}
