//! Admin users management HTTP handlers
//!
//! These endpoints manage admin users for the RaisinDB admin console.
//! All endpoints require authentication via JWT token.
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use raisin_models::admin_user::{AdminAccessFlags, DatabaseAdminUser};
use raisin_rocksdb::AdminClaims;
use serde::{Deserialize, Serialize};

use crate::{error::ApiError, state::AppState};

/// Request to create a new admin user
#[derive(Debug, Deserialize)]
pub struct CreateAdminUserRequest {
    pub username: String,
    pub email: Option<String>,
    pub password: String,
    pub access_flags: AdminAccessFlags,
}

/// Response for an admin user
#[derive(Debug, Serialize)]
pub struct AdminUserResponse {
    pub user_id: String,
    pub username: String,
    pub email: Option<String>,
    pub tenant_id: String,
    pub access_flags: AdminAccessFlags,
    pub must_change_password: bool,
    pub created_at: String,
    pub last_login: Option<String>,
    pub is_active: bool,
}

impl From<DatabaseAdminUser> for AdminUserResponse {
    fn from(user: DatabaseAdminUser) -> Self {
        Self {
            user_id: user.user_id,
            username: user.username,
            email: user.email,
            tenant_id: user.tenant_id,
            access_flags: user.access_flags,
            must_change_password: user.must_change_password,
            created_at: user.created_at.to_rfc3339(),
            last_login: user.last_login.map(|dt| dt.to_rfc3339()),
            is_active: user.is_active,
        }
    }
}

/// Request to update an admin user
#[derive(Debug, Deserialize)]
pub struct UpdateAdminUserRequest {
    pub email: Option<String>,
    pub access_flags: Option<AdminAccessFlags>,
    pub must_change_password: Option<bool>,
    pub is_active: Option<bool>,
}

/// List all admin users for a tenant
///
/// # Endpoint
/// GET /api/raisindb/sys/{tenant_id}/admin-users
///
/// # Headers
/// Authorization: Bearer {jwt_token}
///
/// # Response
/// ```json
/// [
///   {
///     "user_id": "uuid",
///     "username": "admin",
///     "email": "admin@example.com",
///     "tenant_id": "default",
///     "access_flags": {...},
///     "must_change_password": false,
///     "created_at": "2024-01-01T00:00:00Z",
///     "updated_at": "2024-01-01T00:00:00Z"
///   }
/// ]
/// ```
#[cfg(feature = "storage-rocksdb")]
pub async fn list_admin_users(
    State(state): State<AppState>,
    Path(tenant_id): Path<String>,
    Extension(claims): Extension<AdminClaims>,
) -> Result<Json<Vec<AdminUserResponse>>, ApiError> {
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

    let auth_service = state
        .auth_service()
        .ok_or_else(|| ApiError::internal("Authentication service not available"))?;

    let users = auth_service
        .list_users(&tenant_id)
        .map_err(ApiError::from)?;

    let response: Vec<AdminUserResponse> = users.into_iter().map(AdminUserResponse::from).collect();

    Ok(Json(response))
}

/// Get a specific admin user
///
/// # Endpoint
/// GET /api/raisindb/sys/{tenant_id}/admin-users/{username}
///
/// # Headers
/// Authorization: Bearer {jwt_token}
#[cfg(feature = "storage-rocksdb")]
pub async fn get_admin_user(
    State(state): State<AppState>,
    Path((tenant_id, username)): Path<(String, String)>,
    Extension(claims): Extension<AdminClaims>,
) -> Result<Json<AdminUserResponse>, ApiError> {
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

    let auth_service = state
        .auth_service()
        .ok_or_else(|| ApiError::internal("Authentication service not available"))?;

    let user = auth_service
        .get_user(&tenant_id, &username)
        .map_err(ApiError::from)?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "USER_NOT_FOUND",
                "Admin user not found",
            )
        })?;

    Ok(Json(AdminUserResponse::from(user)))
}

/// Create a new admin user
///
/// # Endpoint
/// POST /api/raisindb/sys/{tenant_id}/admin-users
///
/// # Headers
/// Authorization: Bearer {jwt_token}
///
/// # Body
/// ```json
/// {
///   "username": "newadmin",
///   "email": "newadmin@example.com",
///   "password": "secure_password",
///   "access_flags": {
///     "admin_console": true,
///     "cli": false,
///     "api": false
///   }
/// }
/// ```
#[cfg(feature = "storage-rocksdb")]
pub async fn create_admin_user(
    State(state): State<AppState>,
    Path(tenant_id): Path<String>,
    Extension(claims): Extension<AdminClaims>,
    Json(req): Json<CreateAdminUserRequest>,
) -> Result<(StatusCode, Json<AdminUserResponse>), ApiError> {
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

    let auth_service = state
        .auth_service()
        .ok_or_else(|| ApiError::internal("Authentication service not available"))?;

    let user = auth_service
        .create_user(
            tenant_id,
            req.username,
            req.email,
            req.password,
            req.access_flags,
        )
        .map_err(ApiError::from)?;

    Ok((StatusCode::CREATED, Json(AdminUserResponse::from(user))))
}

/// Update an admin user
///
/// # Endpoint
/// PUT /api/raisindb/sys/{tenant_id}/admin-users/{username}
///
/// # Headers
/// Authorization: Bearer {jwt_token}
///
/// # Body
/// ```json
/// {
///   "email": "updated@example.com",
///   "access_flags": {
///     "admin_console": true,
///     "cli": true,
///     "api": false
///   },
///   "must_change_password": false
/// }
/// ```
#[cfg(feature = "storage-rocksdb")]
pub async fn update_admin_user(
    State(state): State<AppState>,
    Path((tenant_id, username)): Path<(String, String)>,
    Extension(claims): Extension<AdminClaims>,
    Json(req): Json<UpdateAdminUserRequest>,
) -> Result<Json<AdminUserResponse>, ApiError> {
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

    let auth_service = state
        .auth_service()
        .ok_or_else(|| ApiError::internal("Authentication service not available"))?;

    // Get existing user
    let mut user = auth_service
        .get_user(&tenant_id, &username)
        .map_err(ApiError::from)?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "USER_NOT_FOUND",
                "Admin user not found",
            )
        })?;

    // Update fields if provided
    if let Some(email) = req.email {
        user.email = Some(email);
    }
    if let Some(access_flags) = req.access_flags {
        user.access_flags = access_flags;
    }
    if let Some(must_change_password) = req.must_change_password {
        user.must_change_password = must_change_password;
    }
    if let Some(is_active) = req.is_active {
        user.is_active = is_active;
    }

    // Save updated user
    auth_service.update_user(&user).map_err(ApiError::from)?;

    Ok(Json(AdminUserResponse::from(user)))
}

/// Delete an admin user
///
/// # Endpoint
/// DELETE /api/raisindb/sys/{tenant_id}/admin-users/{username}
///
/// # Headers
/// Authorization: Bearer {jwt_token}
#[cfg(feature = "storage-rocksdb")]
pub async fn delete_admin_user(
    State(state): State<AppState>,
    Path((tenant_id, username)): Path<(String, String)>,
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

    // Prevent users from deleting themselves
    if claims.username == username {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "CANNOT_DELETE_SELF",
            "Cannot delete your own user account",
        ));
    }

    let auth_service = state
        .auth_service()
        .ok_or_else(|| ApiError::internal("Authentication service not available"))?;

    auth_service
        .delete_user(&tenant_id, &username)
        .map_err(ApiError::from)?;

    Ok(StatusCode::NO_CONTENT)
}
