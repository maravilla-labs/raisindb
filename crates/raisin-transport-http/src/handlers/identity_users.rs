//! Identity users management HTTP handlers
//!
//! These endpoints manage identity users (users registered via /auth/{repo}/register).
//! All endpoints require admin authentication via JWT token.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
#[cfg(feature = "storage-rocksdb")]
use raisin_rocksdb::repositories::IdentityRepository;
use raisin_rocksdb::AdminClaims;
use serde::{Deserialize, Serialize};

use crate::{error::ApiError, state::AppState};

/// Response for an identity user
#[derive(Debug, Serialize)]
pub struct IdentityUserResponse {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub email_verified: bool,
    pub is_active: bool,
    pub linked_providers: Vec<String>,
    pub created_at: String,
    pub updated_at: Option<String>,
    pub last_login_at: Option<String>,
}

impl From<raisin_models::auth::Identity> for IdentityUserResponse {
    fn from(identity: raisin_models::auth::Identity) -> Self {
        Self {
            id: identity.identity_id,
            email: identity.email,
            display_name: identity.display_name,
            avatar_url: identity.avatar_url,
            email_verified: identity.email_verified,
            is_active: identity.is_active,
            linked_providers: identity
                .linked_providers
                .iter()
                .map(|p| p.strategy_id.clone())
                .collect(),
            created_at: identity.created_at.to_rfc3339(),
            updated_at: identity.updated_at.map(|t| t.to_rfc3339()),
            last_login_at: identity.last_login_at.map(|t| t.to_rfc3339()),
        }
    }
}

/// Query parameters for listing identity users
#[derive(Debug, Deserialize)]
pub struct ListIdentityUsersQuery {
    /// Page number (1-indexed, default: 1)
    #[serde(default = "default_page")]
    pub page: u32,
    /// Items per page (default: 50, max: 100)
    #[serde(default = "default_per_page")]
    pub per_page: u32,
    /// Filter by email (partial match)
    pub email: Option<String>,
    /// Filter by active status
    pub is_active: Option<bool>,
}

fn default_page() -> u32 {
    1
}

fn default_per_page() -> u32 {
    50
}

/// Request to update an identity user
#[derive(Debug, Deserialize)]
pub struct UpdateIdentityUserRequest {
    pub display_name: Option<String>,
    pub is_active: Option<bool>,
    pub email_verified: Option<bool>,
}

/// List all identity users for a tenant
///
/// # Endpoint
/// GET /api/raisindb/sys/{tenant_id}/identity-users
///
/// # Headers
/// Authorization: Bearer {jwt_token}
#[cfg(feature = "storage-rocksdb")]
pub async fn list_identity_users(
    State(state): State<AppState>,
    Path(tenant_id): Path<String>,
    Extension(claims): Extension<AdminClaims>,
    Query(query): Query<ListIdentityUsersQuery>,
) -> Result<Json<Vec<IdentityUserResponse>>, ApiError> {
    // Verify tenant_id matches the token
    if claims.tenant_id != tenant_id {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "TENANT_MISMATCH",
            "Token tenant does not match requested tenant",
        ));
    }

    // Check if user has console_login access
    if !claims.access_flags.console_login {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "INSUFFICIENT_PERMISSIONS",
            "Admin console access required",
        ));
    }

    let rocksdb_storage = state.rocksdb_storage.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "STORAGE_NOT_AVAILABLE",
            "RocksDB storage not available",
        )
    })?;

    let db = rocksdb_storage.db().clone();
    let operation_capture = rocksdb_storage.operation_capture().clone();
    let identity_repo = IdentityRepository::new(db, operation_capture);

    // Cap per_page at 100
    let per_page = query.per_page.min(100);
    let offset = (query.page.saturating_sub(1)) * per_page;

    let identities = identity_repo
        .list(&tenant_id, per_page as usize, offset as usize)
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "DATABASE_ERROR",
                format!("Failed to list identities: {}", e),
            )
        })?;

    // Apply filters (email, is_active)
    let filtered: Vec<IdentityUserResponse> = identities
        .into_iter()
        .filter(|i| {
            if let Some(ref email_filter) = query.email {
                if !i
                    .email
                    .to_lowercase()
                    .contains(&email_filter.to_lowercase())
                {
                    return false;
                }
            }
            if let Some(is_active_filter) = query.is_active {
                if i.is_active != is_active_filter {
                    return false;
                }
            }
            true
        })
        .map(IdentityUserResponse::from)
        .collect();

    Ok(Json(filtered))
}

/// Get a specific identity user
///
/// # Endpoint
/// GET /api/raisindb/sys/{tenant_id}/identity-users/{identity_id}
///
/// # Headers
/// Authorization: Bearer {jwt_token}
#[cfg(feature = "storage-rocksdb")]
pub async fn get_identity_user(
    State(state): State<AppState>,
    Path((tenant_id, identity_id)): Path<(String, String)>,
    Extension(claims): Extension<AdminClaims>,
) -> Result<Json<IdentityUserResponse>, ApiError> {
    // Verify tenant_id matches the token
    if claims.tenant_id != tenant_id {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "TENANT_MISMATCH",
            "Token tenant does not match requested tenant",
        ));
    }

    // Check if user has console_login access
    if !claims.access_flags.console_login {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "INSUFFICIENT_PERMISSIONS",
            "Admin console access required",
        ));
    }

    let rocksdb_storage = state.rocksdb_storage.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "STORAGE_NOT_AVAILABLE",
            "RocksDB storage not available",
        )
    })?;

    let db = rocksdb_storage.db().clone();
    let operation_capture = rocksdb_storage.operation_capture().clone();
    let identity_repo = IdentityRepository::new(db, operation_capture);

    let identity = identity_repo
        .get(&tenant_id, &identity_id)
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "DATABASE_ERROR",
                format!("Failed to get identity: {}", e),
            )
        })?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "IDENTITY_NOT_FOUND",
                "Identity user not found",
            )
        })?;

    Ok(Json(IdentityUserResponse::from(identity)))
}

/// Update an identity user
///
/// # Endpoint
/// PATCH /api/raisindb/sys/{tenant_id}/identity-users/{identity_id}
///
/// # Headers
/// Authorization: Bearer {jwt_token}
#[cfg(feature = "storage-rocksdb")]
pub async fn update_identity_user(
    State(state): State<AppState>,
    Path((tenant_id, identity_id)): Path<(String, String)>,
    Extension(claims): Extension<AdminClaims>,
    Json(req): Json<UpdateIdentityUserRequest>,
) -> Result<Json<IdentityUserResponse>, ApiError> {
    // Verify tenant_id matches the token
    if claims.tenant_id != tenant_id {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "TENANT_MISMATCH",
            "Token tenant does not match requested tenant",
        ));
    }

    // Check if user has console_login access
    if !claims.access_flags.console_login {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "INSUFFICIENT_PERMISSIONS",
            "Admin console access required",
        ));
    }

    let rocksdb_storage = state.rocksdb_storage.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "STORAGE_NOT_AVAILABLE",
            "RocksDB storage not available",
        )
    })?;

    let db = rocksdb_storage.db().clone();
    let operation_capture = rocksdb_storage.operation_capture().clone();
    let identity_repo = IdentityRepository::new(db, operation_capture);

    // Get existing identity
    let mut identity = identity_repo
        .get(&tenant_id, &identity_id)
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "DATABASE_ERROR",
                format!("Failed to get identity: {}", e),
            )
        })?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "IDENTITY_NOT_FOUND",
                "Identity user not found",
            )
        })?;

    // Update fields if provided
    if let Some(display_name) = req.display_name {
        identity.display_name = Some(display_name);
    }
    if let Some(is_active) = req.is_active {
        identity.is_active = is_active;
    }
    if let Some(email_verified) = req.email_verified {
        identity.email_verified = email_verified;
    }

    // Update the updated_at timestamp
    identity.updated_at = Some(raisin_models::timestamp::StorageTimestamp::now());

    // Save updated identity
    identity_repo
        .upsert(&tenant_id, &identity, "admin:update")
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "DATABASE_ERROR",
                format!("Failed to update identity: {}", e),
            )
        })?;

    Ok(Json(IdentityUserResponse::from(identity)))
}

/// Delete an identity user
///
/// # Endpoint
/// DELETE /api/raisindb/sys/{tenant_id}/identity-users/{identity_id}
///
/// # Headers
/// Authorization: Bearer {jwt_token}
#[cfg(feature = "storage-rocksdb")]
pub async fn delete_identity_user(
    State(state): State<AppState>,
    Path((tenant_id, identity_id)): Path<(String, String)>,
    Extension(claims): Extension<AdminClaims>,
) -> Result<StatusCode, ApiError> {
    // Verify tenant_id matches the token
    if claims.tenant_id != tenant_id {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "TENANT_MISMATCH",
            "Token tenant does not match requested tenant",
        ));
    }

    // Check if user has console_login access
    if !claims.access_flags.console_login {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "INSUFFICIENT_PERMISSIONS",
            "Admin console access required",
        ));
    }

    let rocksdb_storage = state.rocksdb_storage.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "STORAGE_NOT_AVAILABLE",
            "RocksDB storage not available",
        )
    })?;

    let db = rocksdb_storage.db().clone();
    let operation_capture = rocksdb_storage.operation_capture().clone();
    let identity_repo = IdentityRepository::new(db, operation_capture);

    // Delete identity (this also deletes associated sessions)
    identity_repo
        .delete(&tenant_id, &identity_id, "admin:delete")
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "DATABASE_ERROR",
                format!("Failed to delete identity: {}", e),
            )
        })?;

    Ok(StatusCode::NO_CONTENT)
}
