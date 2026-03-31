// SPDX-License-Identifier: BSL-1.1

//! Repository management operation handlers

use parking_lot::RwLock;
use raisin_context::RepositoryConfig;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{
    BranchRepository, RegistryRepository, RepositoryManagementRepository, Storage,
};
use std::sync::Arc;

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{
        RepositoryCreatePayload, RepositoryDeletePayload, RepositoryGetPayload,
        RepositoryListPayload, RepositoryUpdatePayload, RequestEnvelope, ResponseEnvelope,
    },
};

/// Handle repository creation
pub async fn handle_repository_create<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: RepositoryCreatePayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo_mgmt = state.storage.repository_management();

    // Ensure tenant is registered (will emit TenantCreated event if new)
    let registry = state.storage.registry();
    registry
        .register_tenant(tenant_id, std::collections::HashMap::new())
        .await?;

    // Check if repository already exists
    if repo_mgmt
        .repository_exists(tenant_id, &payload.repository_id)
        .await?
    {
        return Err(WsError::InvalidRequest(format!(
            "Repository already exists: {}",
            payload.repository_id
        )));
    }

    // Extract configuration from payload
    let config: RepositoryConfig = if let Some(config_value) = payload.config {
        serde_json::from_value(config_value).map_err(|e| {
            WsError::InvalidRequest(format!("Invalid repository configuration: {}", e))
        })?
    } else {
        // Default configuration
        RepositoryConfig {
            default_branch: "main".to_string(),
            description: payload.description,
            tags: std::collections::HashMap::new(),
            default_language: "en".to_string(),
            supported_languages: vec!["en".to_string()],
            locale_fallback_chains: std::collections::HashMap::new(),
        }
    };

    let default_branch = config.default_branch.clone();

    // Create the repository
    let repo = repo_mgmt
        .create_repository(tenant_id, &payload.repository_id, config)
        .await?;

    // Create the default branch
    let branches = state.storage.branches();
    if let Err(e) = branches
        .create_branch(
            tenant_id,
            &payload.repository_id,
            &default_branch,
            "system", // created_by
            None,     // from_revision - start from scratch
            None,     // upstream_branch - main has no upstream
            false,    // protected
            false,    // include_revision_history - not applicable for new repo
        )
        .await
    {
        tracing::warn!(
            "Failed to create default branch '{}' for repository '{}': {}",
            default_branch,
            payload.repository_id,
            e
        );
        // Don't fail the repository creation if branch creation fails
    } else {
        tracing::info!(
            "Created default branch '{}' for repository '{}'",
            default_branch,
            payload.repository_id
        );
    }

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(repo)?,
    )))
}

/// Handle repository get
pub async fn handle_repository_get<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage,
    B: raisin_binary::BinaryStorage,
{
    let payload: RepositoryGetPayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo_mgmt = state.storage.repository_management();

    let repo = repo_mgmt
        .get_repository(tenant_id, &payload.repository_id)
        .await?
        .ok_or_else(|| {
            WsError::InvalidRequest(format!("Repository not found: {}", payload.repository_id))
        })?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(repo)?,
    )))
}

/// Handle repository list
pub async fn handle_repository_list<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage,
    B: raisin_binary::BinaryStorage,
{
    let _payload: RepositoryListPayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo_mgmt = state.storage.repository_management();

    let repos = repo_mgmt.list_repositories_for_tenant(tenant_id).await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(repos)?,
    )))
}

/// Handle repository update
pub async fn handle_repository_update<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: RepositoryUpdatePayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo_mgmt = state.storage.repository_management();

    // Get existing repository to preserve unchanged fields
    let existing = repo_mgmt
        .get_repository(tenant_id, &payload.repository_id)
        .await?
        .ok_or_else(|| {
            WsError::InvalidRequest(format!("Repository not found: {}", payload.repository_id))
        })?;

    // Extract updated configuration from payload or use existing
    let config: RepositoryConfig = if let Some(config_value) = payload.config {
        let mut new_config: RepositoryConfig =
            serde_json::from_value(config_value).map_err(|e| {
                WsError::InvalidRequest(format!("Invalid repository configuration: {}", e))
            })?;

        // Preserve immutable fields
        new_config.default_language = existing.config.default_language;
        new_config
    } else {
        // If no config provided, update only description if present
        RepositoryConfig {
            default_branch: existing.config.default_branch,
            description: payload.description.or(existing.config.description),
            tags: existing.config.tags,
            default_language: existing.config.default_language, // IMMUTABLE
            supported_languages: existing.config.supported_languages,
            locale_fallback_chains: existing.config.locale_fallback_chains,
        }
    };

    repo_mgmt
        .update_repository_config(tenant_id, &payload.repository_id, config)
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::json!({"success": true}),
    )))
}

/// Handle repository deletion
pub async fn handle_repository_delete<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: RepositoryDeletePayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo_mgmt = state.storage.repository_management();

    let deleted = repo_mgmt
        .delete_repository(tenant_id, &payload.repository_id)
        .await?;

    if deleted {
        Ok(Some(ResponseEnvelope::success(
            request.request_id,
            serde_json::json!({"success": true}),
        )))
    } else {
        Err(WsError::InvalidRequest(format!(
            "Repository not found: {}",
            payload.repository_id
        )))
    }
}
