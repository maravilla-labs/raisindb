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

/// Request to create a new identity user (admin action)
#[derive(Debug, Deserialize)]
pub struct CreateIdentityUserRequest {
    pub email: String,
    pub password: String,
    pub display_name: Option<String>,
    /// If true, mark email as verified immediately
    pub email_verified: Option<bool>,
    /// Repository IDs to create raisin:User nodes in
    pub repos: Option<Vec<String>>,
    /// Default role_ids for created user nodes (defaults to ["viewer", "authenticated_user"])
    pub default_roles: Option<Vec<String>>,
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

    // Track original active status for cascade detection
    let was_active = identity.is_active;

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

    let active_changed = identity.is_active != was_active;

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

    // Cascade active status change to repository user nodes
    if active_changed {
        let status = if identity.is_active { "active" } else { "inactive" };
        cascade_user_status(rocksdb_storage, &tenant_id, &identity_id, status).await;
    }

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

    // Soft-delete linked repository user nodes before deleting the identity
    cascade_user_status(rocksdb_storage, &tenant_id, &identity_id, "deleted").await;

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

/// Cascade a status change to all raisin:User nodes linked to an identity across repos.
///
/// This finds all repositories for the tenant, then for each repo finds the
/// raisin:User node with `user_id` = identity_id and updates its `status` property.
#[cfg(feature = "storage-rocksdb")]
async fn cascade_user_status(
    storage: &std::sync::Arc<raisin_rocksdb::RocksDBStorage>,
    tenant_id: &str,
    identity_id: &str,
    new_status: &str,
) {
    use raisin_core::services::node_service::NodeService;
    use raisin_models::auth::AuthContext;
    use raisin_models::nodes::properties::PropertyValue;
    use raisin_storage::{NodeRepository, RepositoryManagementRepository, Storage};

    let repos = match Storage::repository_management(storage.as_ref())
        .list_repositories_for_tenant(tenant_id)
        .await
    {
        Ok(repos) => repos,
        Err(e) => {
            tracing::warn!(
                tenant_id = tenant_id,
                error = %e,
                "Failed to list repos for identity cascade"
            );
            return;
        }
    };

    let identity_id_value = PropertyValue::String(identity_id.to_string());
    let workspace = "raisin:access_control";

    for repo in &repos {
        let branch = repo.config.default_branch.as_str();
        let scope = raisin_storage::StorageScope::new(tenant_id, &repo.repo_id, branch, workspace);

        // Find user node linked to this identity
        let nodes = match Storage::nodes(storage.as_ref())
            .find_by_property(scope, "user_id", &identity_id_value)
            .await
        {
            Ok(nodes) => nodes,
            Err(e) => {
                tracing::warn!(
                    repo_id = %repo.repo_id,
                    error = %e,
                    "Failed to find user node for cascade"
                );
                continue;
            }
        };

        let user_node = match nodes.into_iter().find(|n| n.node_type == "raisin:User") {
            Some(node) => node,
            None => continue,
        };

        // Update the status property
        let node_service: NodeService<raisin_rocksdb::RocksDBStorage> =
            NodeService::new_with_context(
                storage.clone(),
                tenant_id.to_string(),
                repo.repo_id.clone(),
                branch.to_string(),
                workspace.to_string(),
            )
            .with_auth(AuthContext::system());

        if let Err(e) = node_service
            .update_property_by_path(
                &user_node.path,
                "status",
                PropertyValue::String(new_status.to_string()),
            )
            .await
        {
            tracing::warn!(
                repo_id = %repo.repo_id,
                user_path = %user_node.path,
                error = %e,
                "Failed to cascade status to user node"
            );
        } else {
            tracing::info!(
                identity_id = identity_id,
                repo_id = %repo.repo_id,
                user_path = %user_node.path,
                new_status = new_status,
                "Cascaded status to repository user node"
            );
        }
    }
}

/// Create a new identity user (admin action)
///
/// # Endpoint
/// POST /api/raisindb/sys/{tenant_id}/identity-users
///
/// # Headers
/// Authorization: Bearer {jwt_token}
///
/// Optionally creates raisin:User nodes in specified repositories.
#[cfg(feature = "storage-rocksdb")]
pub async fn create_identity_user(
    State(state): State<AppState>,
    Path(tenant_id): Path<String>,
    Extension(claims): Extension<AdminClaims>,
    Json(req): Json<CreateIdentityUserRequest>,
) -> Result<(StatusCode, Json<IdentityUserResponse>), ApiError> {
    use raisin_models::auth::{Identity, LocalCredentials};
    use raisin_models::timestamp::StorageTimestamp;
    use uuid::Uuid;

    use crate::handlers::identity_auth::helpers::{validate_email, validate_password};
    use crate::handlers::identity_auth::user_node::ensure_user_node;

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

    // Validate inputs
    validate_email(&req.email)?;
    validate_password(&req.password)?;

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

    // Check if email already exists
    if identity_repo
        .find_by_email(&tenant_id, &req.email)
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "DATABASE_ERROR",
                format!("Failed to check existing identity: {}", e),
            )
        })?
        .is_some()
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "EMAIL_EXISTS",
            "An account with this email already exists",
        ));
    }

    // Create new identity
    let identity_id = Uuid::new_v4().to_string();
    let mut identity = Identity::new(
        identity_id.clone(),
        tenant_id.clone(),
        req.email.clone(),
    );
    identity.display_name = req.display_name.clone();
    if req.email_verified.unwrap_or(false) {
        identity.email_verified = true;
    }

    // Hash password and set local credentials
    let password_hash = IdentityRepository::hash_password(&req.password).map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "PASSWORD_HASH_ERROR",
            format!("Failed to hash password: {}", e),
        )
    })?;
    identity.local_credentials = Some(LocalCredentials::new(password_hash));
    identity.updated_at = Some(StorageTimestamp::now());

    // Save identity
    identity_repo
        .upsert(&tenant_id, &identity, "admin:create")
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "DATABASE_ERROR",
                format!("Failed to create identity: {}", e),
            )
        })?;

    // Create user nodes in specified repositories
    let repos = req.repos.unwrap_or_default();
    let default_roles = req.default_roles.unwrap_or_else(|| {
        vec!["viewer".to_string(), "authenticated_user".to_string()]
    });

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
            Ok(path) => {
                tracing::info!(
                    identity_id = %identity_id,
                    repo_id = %repo_id,
                    home = %path,
                    "User node created for admin-created identity"
                );
            }
            Err(e) => {
                tracing::warn!(
                    identity_id = %identity_id,
                    repo_id = %repo_id,
                    error = %e,
                    "Failed to create user node for admin-created identity"
                );
            }
        }
    }

    Ok((StatusCode::CREATED, Json(IdentityUserResponse::from(identity))))
}

/// Request to link an identity user to an existing repository user node
#[derive(Debug, Deserialize)]
pub struct LinkIdentityUserRequest {
    pub repo_id: String,
    pub user_node_path: String,
}

/// Link an identity user to an existing repository user node
///
/// # Endpoint
/// POST /api/raisindb/sys/{tenant_id}/identity-users/{identity_id}/link
///
/// Sets the `user_id` property on the target raisin:User node to the identity_id.
#[cfg(feature = "storage-rocksdb")]
pub async fn link_identity_user(
    State(state): State<AppState>,
    Path((tenant_id, identity_id)): Path<(String, String)>,
    Extension(claims): Extension<AdminClaims>,
    Json(req): Json<LinkIdentityUserRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    use raisin_core::services::node_service::NodeService;
    use raisin_models::auth::AuthContext;
    use raisin_models::nodes::properties::PropertyValue;
    use raisin_storage::{NodeRepository, RepositoryManagementRepository, Storage};

    // Verify tenant_id matches the token
    if claims.tenant_id != tenant_id {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "TENANT_MISMATCH",
            "Token tenant does not match requested tenant",
        ));
    }

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

    // Verify identity exists
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

    // Get default branch
    let default_branch = match Storage::repository_management(rocksdb_storage.as_ref())
        .get_repository(&tenant_id, &req.repo_id)
        .await
    {
        Ok(Some(repo)) => repo.config.default_branch.clone(),
        _ => "main".to_string(),
    };

    let workspace = "raisin:access_control";
    let node_service: NodeService<raisin_rocksdb::RocksDBStorage> = NodeService::new_with_context(
        rocksdb_storage.clone(),
        tenant_id.clone(),
        req.repo_id.clone(),
        default_branch,
        workspace.to_string(),
    )
    .with_auth(AuthContext::system());

    // Check the target node exists and is a raisin:User
    let target_node = node_service
        .get_by_path(&req.user_node_path)
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "DATABASE_ERROR",
                format!("Failed to get user node: {}", e),
            )
        })?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "USER_NODE_NOT_FOUND",
                format!("No node found at path: {}", req.user_node_path),
            )
        })?;

    if target_node.node_type != "raisin:User" {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "NOT_A_USER_NODE",
            format!(
                "Node at {} is type '{}', expected 'raisin:User'",
                req.user_node_path, target_node.node_type
            ),
        ));
    }

    // Check no other user node already links to this identity
    let identity_id_value = PropertyValue::String(identity_id.clone());
    let existing_links = raisin_storage::Storage::nodes(rocksdb_storage.as_ref())
        .find_by_property(
            raisin_storage::StorageScope::new(&tenant_id, &req.repo_id, "main", workspace),
            "user_id",
            &identity_id_value,
        )
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "DATABASE_ERROR",
                format!("Failed to check existing links: {}", e),
            )
        })?;

    let already_linked = existing_links
        .iter()
        .find(|n| n.node_type == "raisin:User" && n.id != target_node.id);

    if let Some(existing) = already_linked {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "IDENTITY_ALREADY_LINKED",
            format!(
                "Identity is already linked to user node at '{}'",
                existing.path
            ),
        ));
    }

    // Update the user node with identity link
    node_service
        .update_property_by_path(
            &req.user_node_path,
            "user_id",
            PropertyValue::String(identity_id.clone()),
        )
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "DATABASE_ERROR",
                format!("Failed to update user_id: {}", e),
            )
        })?;

    // Sync email from identity
    node_service
        .update_property_by_path(
            &req.user_node_path,
            "email",
            PropertyValue::String(identity.email.clone()),
        )
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "DATABASE_ERROR",
                format!("Failed to update email: {}", e),
            )
        })?;

    tracing::info!(
        identity_id = %identity_id,
        repo_id = %req.repo_id,
        user_node_path = %req.user_node_path,
        "Identity linked to repository user node"
    );

    Ok(Json(serde_json::json!({
        "identity_id": identity_id,
        "email": identity.email,
        "repo_id": req.repo_id,
        "user_node_path": req.user_node_path,
        "linked": true
    })))
}
