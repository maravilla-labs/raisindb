// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! User profile and API key management HTTP handlers
//!
//! These endpoints allow authenticated users to:
//! - View their profile information
//! - Manage their API keys (create, list, revoke)
//! - View available repositories (for PostgreSQL connection strings)

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use raisin_models::admin_user::AdminAccessFlags;
use raisin_models::api_key::{ApiKeyResponse, CreateApiKeyRequest, CreateApiKeyResponse};
use raisin_rocksdb::AdminClaims;
use serde::Serialize;

use crate::{error::ApiError, state::AppState};

/// Response for user profile
#[derive(Debug, Serialize)]
pub struct UserProfileResponse {
    pub user_id: String,
    pub username: String,
    pub email: Option<String>,
    pub tenant_id: String,
    pub access_flags: AdminAccessFlags,
    pub must_change_password: bool,
}

/// Get the current user's profile
///
/// # Endpoint
/// GET /api/raisindb/me
///
/// # Headers
/// Authorization: Bearer {jwt_token}
///
/// # Response
/// ```json
/// {
///   "user_id": "uuid",
///   "username": "admin",
///   "email": "admin@example.com",
///   "tenant_id": "default",
///   "access_flags": {
///     "console_login": true,
///     "cli_access": true,
///     "api_access": true,
///     "pgwire_access": false
///   },
///   "must_change_password": false
/// }
/// ```
pub async fn get_profile(
    State(state): State<AppState>,
    Extension(claims): Extension<AdminClaims>,
) -> Result<Json<UserProfileResponse>, ApiError> {
    let auth_service = state
        .auth_service()
        .ok_or_else(|| ApiError::internal("Authentication service not available"))?;

    // Get user from store to ensure we have the latest data
    let user = auth_service
        .get_user(&claims.tenant_id, &claims.username)
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "USER_NOT_FOUND", "User not found"))?;

    Ok(Json(UserProfileResponse {
        user_id: user.user_id,
        username: user.username,
        email: user.email,
        tenant_id: user.tenant_id,
        access_flags: user.access_flags,
        must_change_password: user.must_change_password,
    }))
}

/// List all API keys for the current user
///
/// # Endpoint
/// GET /api/raisindb/me/api-keys
///
/// # Headers
/// Authorization: Bearer {jwt_token}
///
/// # Response
/// ```json
/// [
///   {
///     "key_id": "uuid",
///     "name": "CI/CD Pipeline",
///     "key_prefix": "raisin_ab",
///     "created_at": "2024-01-01T00:00:00Z",
///     "last_used_at": "2024-01-15T00:00:00Z",
///     "is_active": true
///   }
/// ]
/// ```
pub async fn list_api_keys(
    State(state): State<AppState>,
    Extension(claims): Extension<AdminClaims>,
) -> Result<Json<Vec<ApiKeyResponse>>, ApiError> {
    let auth_service = state
        .auth_service()
        .ok_or_else(|| ApiError::internal("Authentication service not available"))?;

    let keys = auth_service
        .list_user_api_keys(&claims.tenant_id, &claims.sub)
        .map_err(ApiError::from)?;

    let response: Vec<ApiKeyResponse> = keys.into_iter().map(ApiKeyResponse::from).collect();

    Ok(Json(response))
}

/// Create a new API key for the current user
///
/// # Endpoint
/// POST /api/raisindb/me/api-keys
///
/// # Headers
/// Authorization: Bearer {jwt_token}
///
/// # Body
/// ```json
/// {
///   "name": "My API Key"
/// }
/// ```
///
/// # Response
/// ```json
/// {
///   "key": {
///     "key_id": "uuid",
///     "name": "My API Key",
///     "key_prefix": "raisin_ab",
///     "created_at": "2024-01-01T00:00:00Z",
///     "last_used_at": null,
///     "is_active": true
///   },
///   "token": "raisin_xxxxxxxxxxxxxxxxxxxxxxxxxxxx"
/// }
/// ```
///
/// **Important**: The `token` field is only returned once at creation time.
/// Users must copy and store it immediately.
pub async fn create_api_key(
    State(state): State<AppState>,
    Extension(claims): Extension<AdminClaims>,
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<(StatusCode, Json<CreateApiKeyResponse>), ApiError> {
    let auth_service = state
        .auth_service()
        .ok_or_else(|| ApiError::internal("Authentication service not available"))?;

    let (api_key, raw_token) = auth_service
        .create_api_key(&claims.tenant_id, &claims.sub, &req.name)
        .map_err(ApiError::from)?;

    Ok((
        StatusCode::CREATED,
        Json(CreateApiKeyResponse {
            key: ApiKeyResponse::from(api_key),
            token: raw_token,
        }),
    ))
}

/// Revoke an API key
///
/// # Endpoint
/// DELETE /api/raisindb/me/api-keys/{key_id}
///
/// # Headers
/// Authorization: Bearer {jwt_token}
///
/// # Response
/// 204 No Content on success
pub async fn revoke_api_key(
    State(state): State<AppState>,
    Extension(claims): Extension<AdminClaims>,
    Path(key_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let auth_service = state
        .auth_service()
        .ok_or_else(|| ApiError::internal("Authentication service not available"))?;

    auth_service
        .revoke_api_key(&claims.tenant_id, &claims.sub, &key_id)
        .map_err(ApiError::from)?;

    Ok(StatusCode::NO_CONTENT)
}

/// List available repositories for the current user's tenant
///
/// # Endpoint
/// GET /api/raisindb/me/repositories
///
/// # Headers
/// Authorization: Bearer {jwt_token}
///
/// # Response
/// ```json
/// ["social_feed_demo", "blog", "docs"]
/// ```
///
/// This endpoint is useful for building PostgreSQL connection strings:
/// `postgresql://{tenant_id}:{api_token}@{host}:5432/{repository}`
pub async fn list_repositories(
    State(state): State<AppState>,
    Extension(claims): Extension<AdminClaims>,
) -> Result<Json<Vec<String>>, ApiError> {
    use raisin_storage::{RepositoryManagementRepository, Storage};

    // Get repositories from storage
    let storage = state.storage();
    let repo_mgmt = storage.repository_management();

    let repos = repo_mgmt
        .list_repositories_for_tenant(&claims.tenant_id)
        .await
        .map_err(ApiError::from)?;

    // Extract just the repo_ids
    let repo_ids: Vec<String> = repos.into_iter().map(|r| r.repo_id).collect();

    Ok(Json(repo_ids))
}
