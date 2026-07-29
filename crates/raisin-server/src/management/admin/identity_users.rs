//! Superadmin handlers for provisioning identity users into a tenant.
//!
//! These are the operator counterpart to the customer-facing
//! `/api/raisindb/sys/{tenant_id}/identity-users` endpoints. The customer
//! surface requires a per-tenant admin JWT, which a hosting control plane
//! does not hold — it holds the superadmin token. Mounting the same
//! capability here lets a control plane provision logins for a tenant it
//! manages without minting or storing per-tenant admin credentials.
//!
//! Deliberately policy-free: the repositories a user is granted access to,
//! the roles they get there, and whether they must change their password on
//! first login are all supplied by the caller. RaisinDB has no opinion about
//! what any given repo or role means to the application on top of it.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use raisin_rocksdb::repositories::IdentityRepository;
use raisin_transport_http::{
    error::ApiError,
    identity_provisioning::{
        ensure_user_node, validate_email, validate_password, IdentityUserResponse,
    },
    state::AppState,
};
use serde::Deserialize;

use crate::management::types::ApiResponse;

/// Roles applied when the caller does not specify any, matching the
/// customer-facing create endpoint.
const DEFAULT_ROLES: [&str; 2] = ["viewer", "authenticated_user"];

/// Cap on a single list page. Provisioning control planes list to render an
/// admin table, not to bulk-export.
const MAX_LIST_PER_PAGE: usize = 500;

#[derive(Debug, Deserialize)]
pub struct CreateIdentityUserRequest {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub display_name: Option<String>,
    /// Mark the email verified immediately. Control planes that already
    /// verified the address (or delivered the password out-of-band to it)
    /// set this to skip the verification round trip.
    #[serde(default)]
    pub email_verified: Option<bool>,
    /// Repositories to create a `raisin:User` node in.
    #[serde(default)]
    pub repos: Option<Vec<String>>,
    /// Role ids granted on each created user node.
    #[serde(default)]
    pub default_roles: Option<Vec<String>>,
    /// Require a password change on first login.
    #[serde(default)]
    pub must_change_password: Option<bool>,
}

/// Create an identity user in a tenant.
///
/// `POST /management/admin/tenants/{tenant_id}/identity-users`
///
/// Creates the `Identity` in the tenant's `raisin:system` workspace and, for
/// each entry in `repos`, a `raisin:User` node carrying `default_roles`.
/// Returns 409 if an identity with this email already exists.
///
/// Repo-node creation is best-effort per repo (a missing repo is logged, not
/// fatal) — matching the customer-facing endpoint, so a partially-configured
/// tenant still yields a usable login.
pub async fn create_identity_user(
    State(app_state): State<AppState>,
    Path(tenant_id): Path<String>,
    Json(req): Json<CreateIdentityUserRequest>,
) -> Result<(StatusCode, Json<ApiResponse<IdentityUserResponse>>), ApiError> {
    use raisin_models::auth::{Identity, LocalCredentials};
    use raisin_models::timestamp::StorageTimestamp;
    use uuid::Uuid;

    validate_email(&req.email)?;
    validate_password(&req.password)?;

    let rocksdb_storage = app_state.rocksdb_storage().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "STORAGE_NOT_AVAILABLE",
            "RocksDB storage not available",
        )
    })?;

    let identity_repo = IdentityRepository::new(
        rocksdb_storage.db().clone(),
        rocksdb_storage.operation_capture().clone(),
    );

    if identity_repo
        .find_by_email(&tenant_id, &req.email)
        .await
        .map_err(|e| ApiError::storage_error(format!("Failed to check existing identity: {}", e)))?
        .is_some()
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "EMAIL_EXISTS",
            "An account with this email already exists",
        ));
    }

    let identity_id = Uuid::new_v4().to_string();
    let mut identity = Identity::new(identity_id.clone(), tenant_id.clone(), req.email.clone());
    identity.display_name = req.display_name.clone();
    if req.email_verified.unwrap_or(false) {
        identity.email_verified = true;
    }

    let password_hash = IdentityRepository::hash_password(&req.password)
        .map_err(|e| ApiError::internal(format!("Failed to hash password: {}", e)))?;
    identity.local_credentials = Some(if req.must_change_password.unwrap_or(false) {
        LocalCredentials::new_with_change_required(password_hash)
    } else {
        LocalCredentials::new(password_hash)
    });
    identity.updated_at = Some(StorageTimestamp::now());

    identity_repo
        .upsert(&tenant_id, &identity, "superadmin:create")
        .await
        .map_err(|e| ApiError::storage_error(format!("Failed to create identity: {}", e)))?;

    let repos = req.repos.unwrap_or_default();
    let default_roles = req
        .default_roles
        .unwrap_or_else(|| DEFAULT_ROLES.iter().map(|r| r.to_string()).collect());

    for repo_id in &repos {
        match ensure_user_node(
            rocksdb_storage,
            &tenant_id,
            repo_id,
            &identity_id,
            &req.email,
            req.display_name.as_deref(),
            &default_roles,
        )
        .await
        {
            Ok(path) => tracing::info!(
                tenant_id = %tenant_id,
                identity_id = %identity_id,
                repo_id = %repo_id,
                home = %path,
                "User node created for superadmin-provisioned identity"
            ),
            Err(e) => tracing::warn!(
                tenant_id = %tenant_id,
                identity_id = %identity_id,
                repo_id = %repo_id,
                error = %e,
                "Failed to create user node for superadmin-provisioned identity"
            ),
        }
    }

    tracing::warn!(
        tenant_id = %tenant_id,
        identity_id = %identity_id,
        "Superadmin provisioned identity user"
    );

    Ok((
        StatusCode::CREATED,
        Json(ApiResponse::ok(IdentityUserResponse::from(identity))),
    ))
}

/// List identity users in a tenant.
///
/// `GET /management/admin/tenants/{tenant_id}/identity-users`
///
/// Never returns credentials — only identity metadata, including whether a
/// password change is still pending.
pub async fn list_identity_users(
    State(app_state): State<AppState>,
    Path(tenant_id): Path<String>,
) -> Result<Json<ApiResponse<Vec<IdentityUserResponse>>>, ApiError> {
    let rocksdb_storage = app_state.rocksdb_storage().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "STORAGE_NOT_AVAILABLE",
            "RocksDB storage not available",
        )
    })?;

    let identity_repo = IdentityRepository::new(
        rocksdb_storage.db().clone(),
        rocksdb_storage.operation_capture().clone(),
    );

    let identities = identity_repo
        .list(&tenant_id, MAX_LIST_PER_PAGE, 0)
        .await
        .map_err(|e| ApiError::storage_error(format!("Failed to list identities: {}", e)))?;

    Ok(Json(ApiResponse::ok(
        identities
            .into_iter()
            .map(IdentityUserResponse::from)
            .collect(),
    )))
}
