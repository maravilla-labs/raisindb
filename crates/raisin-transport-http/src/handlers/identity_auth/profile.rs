// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! User profile handlers (get_me, get_me_for_repo).

use axum::{
    extract::{Path, State},
    Extension, Json,
};

use crate::error::ApiError;
use crate::state::AppState;

use super::types::{AuthProvidersResponse, MeForRepoResponse, MeResponse};

/// Get available authentication providers for a tenant.
///
/// # Endpoint
/// GET /auth/providers
///
/// This returns the list of configured authentication providers for the tenant,
/// allowing the UI to display appropriate login options.
#[cfg(feature = "storage-rocksdb")]
pub async fn get_providers(
    State(_state): State<AppState>,
    Extension(_tenant_info): Extension<crate::middleware::TenantInfo>,
) -> Result<Json<AuthProvidersResponse>, ApiError> {
    // TODO: Load from TenantAuthConfig via IdentityAuthService
    // For now, return a basic response with local auth enabled

    Ok(Json(AuthProvidersResponse {
        providers: vec![],
        local_enabled: true,
        magic_link_enabled: true,
    }))
}

/// Get available authentication providers for a specific repository.
///
/// # Endpoint
/// GET /auth/{repo}/providers
#[cfg(feature = "storage-rocksdb")]
pub async fn get_providers_for_repo(
    State(_state): State<AppState>,
    Extension(_tenant_info): Extension<crate::middleware::TenantInfo>,
    Path(_repo): Path<String>,
) -> Result<Json<AuthProvidersResponse>, ApiError> {
    // TODO: Load from RepoAuthConfig
    // For now, return a basic response with local auth enabled
    Ok(Json(AuthProvidersResponse {
        providers: vec![],
        local_enabled: true,
        magic_link_enabled: true,
    }))
}

/// Get current identity information.
///
/// # Endpoint
/// GET /auth/me
///
/// # Headers
/// Authorization: Bearer {access_token}
///
/// # Response
/// Returns the current user's identity information from the auth context.
/// For anonymous users, returns anonymous: true with a generated ID.
#[cfg(feature = "storage-rocksdb")]
pub async fn get_me(
    State(_state): State<AppState>,
    auth: Option<Extension<raisin_models::auth::AuthContext>>,
) -> Result<Json<MeResponse>, ApiError> {
    // Extract auth context from request extension
    match auth {
        Some(Extension(auth_ctx)) => {
            let user_id = auth_ctx
                .user_id
                .clone()
                .unwrap_or_else(|| "anonymous".to_string());

            Ok(Json(MeResponse {
                id: user_id,
                email: auth_ctx.email.clone(),
                roles: auth_ctx.roles.clone(),
                groups: auth_ctx.groups.clone(),
                anonymous: auth_ctx.user_id.is_none(),
                home: auth_ctx.home.clone(),
            }))
        }
        None => {
            // No auth context - anonymous user
            Ok(Json(MeResponse {
                id: "anonymous".to_string(),
                email: None,
                roles: vec![],
                groups: vec![],
                anonymous: true,
                home: None,
            }))
        }
    }
}

/// Get current user info for a specific repository.
///
/// # Endpoint
/// GET /auth/{repo}/me
///
/// # Headers
/// Authorization: Bearer {access_token}
///
/// # Response
/// Returns the current user's node from the specified repository's
/// access_control workspace, along with identity information.
///
/// For anonymous users, returns anonymous: true with home path to their
/// auto-provisioned node (if JIT provisioning is enabled).
#[cfg(feature = "storage-rocksdb")]
pub async fn get_me_for_repo(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<crate::middleware::TenantInfo>,
    auth: Option<Extension<raisin_models::auth::AuthContext>>,
    Path(repo): Path<String>,
) -> Result<Json<MeForRepoResponse>, ApiError> {
    use raisin_storage::{NodeRepository, RepositoryManagementRepository, Storage, StorageScope};

    let tenant_id = &tenant_info.tenant_id;

    // Extract auth context
    let (user_id, email, roles, anonymous, home) = match auth {
        Some(Extension(auth_ctx)) => (
            auth_ctx
                .user_id
                .clone()
                .unwrap_or_else(|| "anonymous".to_string()),
            auth_ctx.email.clone(),
            auth_ctx.roles.clone(),
            auth_ctx.user_id.is_none(),
            auth_ctx.home.clone(),
        ),
        None => ("anonymous".to_string(), None, vec![], true, None),
    };

    // Get user node if home path is available
    let user_node = if let Some(ref home_path) = home {
        // Get repository's default branch
        let default_branch = state
            .storage
            .repository_management()
            .get_repository(tenant_id, &repo)
            .await
            .ok()
            .flatten()
            .map(|r| r.config.default_branch)
            .unwrap_or_else(|| "main".to_string());

        let workspace = "raisin:access_control";
        let node_repo = state.storage.nodes();

        // Look up node by path
        match node_repo
            .get_by_path(
                StorageScope::new(tenant_id, &repo, &default_branch, workspace),
                home_path,
                None,
            )
            .await
        {
            Ok(Some(node)) => Some(node),
            _ => None,
        }
    } else {
        None
    };

    Ok(Json(MeForRepoResponse {
        id: user_id,
        email,
        roles,
        anonymous,
        home,
        user_node,
    }))
}
