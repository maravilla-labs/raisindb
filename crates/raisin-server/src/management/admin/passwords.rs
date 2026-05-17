//! Superadmin handler for resetting a tenant admin password.

use axum::{extract::State, http::StatusCode, response::Json, Extension};
use raisin_transport_http::{middleware::TenantInfo, state::AppState};
use serde::Serialize;

use crate::management::types::ApiResponse;

#[derive(Debug, Serialize)]
pub struct ResetAdminPasswordResponse {
    pub tenant_id: String,
    pub admin_username: String,
    pub admin_password: String,
}

const ADMIN_USERNAME: &str = "admin";

/// Reset (or set) the admin password for the tenant in the request's `x-tenant-id`.
///
/// Generates a fresh 24-char nanoid password, updates the "admin" user's
/// password hash (or the first listed user if there is no "admin"), and
/// returns the new password.
pub async fn reset_admin_password(
    State(app_state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
) -> Result<Json<ApiResponse<ResetAdminPasswordResponse>>, StatusCode> {
    use raisin_rocksdb::AuthService;

    let auth: std::sync::Arc<AuthService> = app_state
        .auth_service()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .clone();

    let user = match auth
        .get_user(&tenant_info.tenant_id, ADMIN_USERNAME)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        Some(u) => u,
        None => {
            let users = auth
                .list_users(&tenant_info.tenant_id)
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            users.into_iter().next().ok_or(StatusCode::NOT_FOUND)?
        }
    };

    let new_password = nanoid::nanoid!(24);
    let new_hash = AuthService::hash_password(&new_password).map_err(|e| {
        tracing::error!(error = %e, "Failed to hash new admin password");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let mut updated = user.clone();
    updated.password_hash = new_hash;
    auth.update_user(&updated).map_err(|e| {
        tracing::error!(error = %e, "Failed to persist admin password reset");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    tracing::warn!(
        tenant_id = %tenant_info.tenant_id,
        username = %updated.username,
        "Superadmin reset admin password"
    );

    Ok(Json(ApiResponse::ok(ResetAdminPasswordResponse {
        tenant_id: tenant_info.tenant_id,
        admin_username: updated.username,
        admin_password: new_password,
    })))
}
