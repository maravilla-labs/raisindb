//! Scoped API keys, minted by the operator for a tenant's services.
//!
//! `POST /management/admin/tenants/{tenant_id}/api-keys` `{ name, repo, roles }`
//! → 201 `{ success, data: { key_id, token, … } }` (the token is shown once)
//! `GET  /management/admin/tenants/{tenant_id}/api-keys` → the tenant's scoped keys
//! `DELETE /management/admin/tenants/{tenant_id}/api-keys/{key_id}` → 204, also
//! for an unknown key.
//!
//! A scoped key belongs to no admin user and resolves, on HTTP and pgwire
//! alike, to the RLS context of `roles` in `repo`
//! (`PermissionService::resolve_scoped_key`): never the system context, and
//! refused in any other repository and on every management surface. It is
//! what a Studio site's server holds; an admin's own key (`POST
//! /api/raisindb/me/api-keys`) is unchanged.
//!
//! Superadmin only (this subtree), and scoped by the PATH tenant for the same
//! reason as identity-users: `x-tenant-id` falls back to "default".

use std::collections::BTreeSet;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use raisin_models::api_key::{ApiKeyResponse, ApiKeyScope};
use raisin_storage::{RepositoryManagementRepository, Storage};
use raisin_transport_http::{error::ApiError, state::AppState};
use serde::{Deserialize, Serialize};

use crate::management::types::ApiResponse;

/// Branch the access control workspace keeps its roles on.
const ROLES_BRANCH: &str = "main";
const MAX_NAME_LEN: usize = 200;
const MAX_ROLES: usize = 32;

#[derive(Debug, Deserialize)]
pub struct CreateScopedApiKeyRequest {
    /// Shown to operators, e.g. `site:events-acme`.
    pub name: String,
    /// The only repository the key works in.
    pub repo: String,
    /// Role ids of that repository's `raisin:access_control`.
    pub roles: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ScopedApiKeyCreated {
    pub key_id: String,
    /// The raw token (`raisin_…`). Shown ONCE; only its hash is stored.
    pub token: String,
    pub name: String,
    pub key_prefix: String,
    pub repo: String,
    pub roles: Vec<String>,
}

/// The request's roles, trimmed and deduplicated in order, or why not.
pub(crate) fn normalize_request(
    req: &CreateScopedApiKeyRequest,
) -> Result<(String, String, Vec<String>), ApiError> {
    let name = req.name.trim().to_string();
    if name.is_empty() || name.chars().count() > MAX_NAME_LEN {
        return Err(ApiError::validation_failed(format!(
            "`name` must be 1..={MAX_NAME_LEN} characters"
        ))
        .with_field("name"));
    }
    let repo = req.repo.trim().to_string();
    if repo.is_empty() {
        return Err(ApiError::validation_failed("`repo` is required").with_field("repo"));
    }
    let mut seen = BTreeSet::new();
    let roles: Vec<String> = req
        .roles
        .iter()
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty() && seen.insert(r.clone()))
        .collect();
    if roles.is_empty() {
        return Err(ApiError::validation_failed(
            "`roles` must name at least one role; a key with no role could do nothing",
        )
        .with_field("roles"));
    }
    if roles.len() > MAX_ROLES {
        return Err(
            ApiError::validation_failed(format!("at most {MAX_ROLES} roles")).with_field("roles"),
        );
    }
    Ok((name, repo, roles))
}

/// Mint a scoped key.
pub async fn create_scoped_api_key(
    State(app_state): State<AppState>,
    Path(tenant_id): Path<String>,
    Json(req): Json<CreateScopedApiKeyRequest>,
) -> Result<(StatusCode, Json<ApiResponse<ScopedApiKeyCreated>>), ApiError> {
    let (name, repo, roles) = normalize_request(&req)?;
    let storage = app_state.rocksdb_storage().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "STORAGE_NOT_AVAILABLE",
            "RocksDB storage not available",
        )
    })?;
    let auth_service = app_state
        .auth_service()
        .ok_or_else(|| ApiError::internal("auth service not available"))?;

    if storage
        .repository_management()
        .get_repository(&tenant_id, &repo)
        .await?
        .is_none()
    {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "REPOSITORY_NOT_FOUND",
            format!("tenant '{tenant_id}' has no repository '{repo}'"),
        ));
    }

    let permissions = raisin_core::PermissionService::new(storage.clone());
    for role in &roles {
        if !permissions
            .role_exists(&tenant_id, &repo, ROLES_BRANCH, role)
            .await?
        {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "UNKNOWN_ROLE",
                format!("repository '{repo}' has no role '{role}'"),
            )
            .with_field("roles"));
        }
    }

    // Refuse administration in any spelling, judged on the RESOLVED set (with
    // inheritance), not the ids as typed. Re-checked on every use as well.
    let resolved = permissions
        .resolve_for_roles(&tenant_id, &repo, ROLES_BRANCH, "apikey:new", &roles)
        .await?;
    if let Some(reason) = raisin_core::admin_grant_in(&resolved) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "ROLE_GRANTS_ADMIN",
            format!(
                "roles {:?} cannot be given to a scoped key: {reason}",
                roles
            ),
        )
        .with_field("roles"));
    }

    let scope = ApiKeyScope {
        repo: repo.clone(),
        roles: roles.clone(),
    };
    let (key, token) = auth_service
        .create_scoped_api_key(&tenant_id, &name, scope)
        .map_err(|e| ApiError::storage_error(format!("Failed to create API key: {e}")))?;

    tracing::warn!(
        tenant_id = %tenant_id,
        key_id = %key.key_id,
        repo = %repo,
        roles = ?roles,
        "Superadmin minted a scoped API key"
    );

    Ok((
        StatusCode::CREATED,
        Json(ApiResponse::ok(ScopedApiKeyCreated {
            key_id: key.key_id,
            token,
            name,
            key_prefix: key.key_prefix,
            repo,
            roles,
        })),
    ))
}

/// The tenant's scoped keys (revoked ones too, `is_active: false`). No token
/// or hash is ever returned.
pub async fn list_scoped_api_keys(
    State(app_state): State<AppState>,
    Path(tenant_id): Path<String>,
) -> Result<Json<ApiResponse<Vec<ApiKeyResponse>>>, ApiError> {
    let auth_service = app_state
        .auth_service()
        .ok_or_else(|| ApiError::internal("auth service not available"))?;
    let keys = auth_service
        .list_scoped_api_keys(&tenant_id)
        .map_err(|e| ApiError::storage_error(format!("Failed to list API keys: {e}")))?;
    Ok(Json(ApiResponse::ok(
        keys.into_iter().map(ApiKeyResponse::from).collect(),
    )))
}

/// Revoke a scoped key. 204 whether or not it existed: the caller's goal
/// ("this key must not work") holds either way.
pub async fn revoke_scoped_api_key(
    State(app_state): State<AppState>,
    Path((tenant_id, key_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let auth_service = app_state
        .auth_service()
        .ok_or_else(|| ApiError::internal("auth service not available"))?;
    let existed = auth_service
        .revoke_scoped_api_key(&tenant_id, &key_id)
        .map_err(|e| ApiError::storage_error(format!("Failed to revoke API key: {e}")))?;
    tracing::warn!(
        tenant_id = %tenant_id,
        key_id = %key_id,
        existed,
        "Superadmin revoked a scoped API key"
    );
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(name: &str, repo: &str, roles: &[&str]) -> CreateScopedApiKeyRequest {
        CreateScopedApiKeyRequest {
            name: name.into(),
            repo: repo.into(),
            roles: roles.iter().map(|r| r.to_string()).collect(),
        }
    }

    #[test]
    fn requests_are_normalised_or_refused() {
        let (name, repo, roles) = normalize_request(&req(
            " site:a ",
            " studio ",
            &["studio_viewer", " studio_viewer", ""],
        ))
        .unwrap();
        assert_eq!((name.as_str(), repo.as_str()), ("site:a", "studio"));
        assert_eq!(roles, vec!["studio_viewer"]);
        assert!(normalize_request(&req("", "studio", &["r"])).is_err());
        assert!(normalize_request(&req("n", "", &["r"])).is_err());
        assert!(normalize_request(&req("n", "studio", &[])).is_err());
        assert!(normalize_request(&req("n", "studio", &["", " "])).is_err());
    }
}
