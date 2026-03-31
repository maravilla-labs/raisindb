// SPDX-License-Identifier: BSL-1.1

//! Authentication middleware layers.
//!
//! Provides JWT-based authentication middleware supporting both admin and user
//! tokens, including impersonation and anonymous access resolution.

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};

use crate::state::AppState;

use super::path_helpers::extract_repo_from_path;
use super::types::{AuthPrincipal, TenantInfo};

/// Middleware that validates JWT tokens for authentication (dual JWT support).
///
/// Supports both admin and user JWT tokens:
///
/// **Admin tokens (AdminClaims):**
/// - Used by console/API admin users
/// - Can optionally impersonate users via X-Raisin-Impersonate header
/// - Without impersonation, operates as system context (bypasses RLS)
///
/// **User tokens (AuthClaims):**
/// - Used by identity-based authenticated users
/// - Permissions resolved via PermissionService
/// - Subject to workspace-level access control
///
/// Returns 401 Unauthorized if token is missing or invalid.
/// Returns 403 Forbidden if impersonation is denied.
#[cfg(feature = "storage-rocksdb")]
pub async fn require_auth_middleware(
    State(state): State<AppState>,
    mut req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    use raisin_core::PermissionService;
    use raisin_models::auth::AuthContext;

    let auth_header = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    if !auth_header.starts_with("Bearer ") {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let token = &auth_header[7..];

    let auth_service = state
        .auth_service()
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    // Try admin token first, then user token
    let principal = match auth_service.validate_token(token) {
        Ok(admin_claims) => AuthPrincipal::Admin(admin_claims),
        Err(_) => match auth_service.validate_user_token(token) {
            Ok(user_claims) => AuthPrincipal::User(Box::new(user_claims)),
            Err(_) => return Err(StatusCode::UNAUTHORIZED),
        },
    };

    let impersonate_user_id = req
        .headers()
        .get("X-Raisin-Impersonate")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let auth_context = match &principal {
        AuthPrincipal::Admin(admin_claims) => {
            resolve_admin_auth_context(admin_claims, impersonate_user_id, req.uri().path(), &state)
                .await?
        }
        AuthPrincipal::User(user_claims) => {
            if impersonate_user_id.is_some() {
                tracing::warn!(
                    user = %user_claims.sub,
                    "User tokens cannot impersonate - ignoring impersonation header"
                );
            }
            resolve_user_auth_context(user_claims, req.uri().path(), &state).await?
        }
    };

    // Store principal and auth context in request extensions
    match &principal {
        AuthPrincipal::Admin(admin_claims) => {
            req.extensions_mut().insert(admin_claims.clone());
        }
        AuthPrincipal::User(user_claims) => {
            req.extensions_mut().insert(user_claims.as_ref().clone());
        }
    }
    req.extensions_mut().insert(principal);
    req.extensions_mut().insert(auth_context);

    Ok(next.run(req).await)
}

/// Optional auth middleware - extracts auth context if Bearer token present
/// but does not reject requests without auth.
///
/// **Key differences from `require_auth_middleware`:**
/// - If no auth header: proceeds without auth context (public access)
/// - If invalid token: proceeds without auth context
/// - If valid token: resolves permissions and stores auth context
#[cfg(feature = "storage-rocksdb")]
pub async fn optional_auth_middleware(
    State(state): State<AppState>,
    mut req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    use raisin_core::PermissionService;
    use raisin_models::auth::AuthContext;

    let auth_header = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    let Some(auth_header) = auth_header else {
        // No auth header - handle anonymous access
        let tenant_id = req
            .extensions()
            .get::<TenantInfo>()
            .map(|t| t.tenant_id.clone())
            .unwrap_or_else(|| "default".to_string());
        let repo_id =
            extract_repo_from_path(req.uri().path()).unwrap_or_else(|| "default".to_string());

        let anonymous_enabled = is_anonymous_enabled_for_context(
            state.storage(),
            &tenant_id,
            &repo_id,
            state.anonymous_enabled,
        )
        .await;

        if anonymous_enabled {
            let permission_service = PermissionService::new(state.storage().clone());
            let resolved_permissions = permission_service
                .resolve_anonymous_user(&tenant_id, &repo_id, "main")
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(
                        error = %e,
                        "Failed to resolve anonymous user permissions, using empty permissions"
                    );
                    None
                })
                .unwrap_or_else(|| {
                    tracing::warn!("Physical anonymous user not found, using empty permissions");
                    raisin_models::permissions::ResolvedPermissions::anonymous(vec![])
                });

            let user_id = resolved_permissions.user_id.clone();
            let auth_context =
                AuthContext::for_user(&user_id).with_permissions(resolved_permissions);
            tracing::debug!(
                tenant_id = %tenant_id,
                repo_id = %repo_id,
                user_id = %user_id,
                "Auto-authenticating as physical anonymous user (HTTP) with resolved permissions"
            );
            req.extensions_mut().insert(auth_context);
        } else {
            tracing::debug!(
                tenant_id = %tenant_id,
                repo_id = %repo_id,
                "Anonymous access disabled - setting deny-all auth context (HTTP)"
            );
            let deny_context = AuthContext::deny_all();
            req.extensions_mut().insert(deny_context);
        }
        return Ok(next.run(req).await);
    };

    if !auth_header.starts_with("Bearer ") {
        return Ok(next.run(req).await);
    }

    let token = &auth_header[7..];

    let auth_service = match state.auth_service() {
        Some(svc) => svc,
        None => return Ok(next.run(req).await),
    };

    // Try admin token first, then user token
    let principal = match auth_service.validate_token(token) {
        Ok(admin_claims) => {
            tracing::warn!(
                admin_user = %admin_claims.sub,
                "Admin token validated successfully (optional auth)"
            );
            AuthPrincipal::Admin(admin_claims)
        }
        Err(admin_err) => {
            tracing::warn!(
                error = %admin_err,
                "Admin token validation failed, trying user token"
            );
            match auth_service.validate_user_token(token) {
                Ok(user_claims) => {
                    tracing::warn!(
                        user = %user_claims.sub,
                        "User token validated successfully (optional auth)"
                    );
                    AuthPrincipal::User(Box::new(user_claims))
                }
                Err(user_err) => {
                    tracing::warn!(
                        admin_error = %admin_err,
                        user_error = %user_err,
                        "Both token validations failed, proceeding without auth"
                    );
                    return Ok(next.run(req).await);
                }
            }
        }
    };

    let impersonate_user_id = req
        .headers()
        .get("X-Raisin-Impersonate")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let auth_context = match &principal {
        AuthPrincipal::Admin(admin_claims) => {
            resolve_admin_auth_context(admin_claims, impersonate_user_id, req.uri().path(), &state)
                .await?
        }
        AuthPrincipal::User(user_claims) => {
            if impersonate_user_id.is_some() {
                tracing::warn!(
                    user = %user_claims.sub,
                    "User tokens cannot impersonate - ignoring impersonation header"
                );
            }
            resolve_user_auth_context(user_claims, req.uri().path(), &state).await?
        }
    };

    match &principal {
        AuthPrincipal::Admin(admin_claims) => {
            req.extensions_mut().insert(admin_claims.clone());
        }
        AuthPrincipal::User(user_claims) => {
            req.extensions_mut().insert(user_claims.as_ref().clone());
        }
    }
    req.extensions_mut().insert(principal);
    req.extensions_mut().insert(auth_context);

    Ok(next.run(req).await)
}

/// Middleware that validates JWT tokens for ADMIN-ONLY authentication.
///
/// ONLY accepts admin tokens (AdminClaims). Rejects identity user tokens
/// with 403 Forbidden. Does NOT support impersonation.
#[cfg(feature = "storage-rocksdb")]
pub async fn require_admin_auth_middleware(
    State(state): State<AppState>,
    mut req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let auth_header = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    if !auth_header.starts_with("Bearer ") {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let token = &auth_header[7..];

    let auth_service = state
        .auth_service()
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    let admin_claims = match auth_service.validate_token(token) {
        Ok(claims) => claims,
        Err(_) => {
            if auth_service.validate_user_token(token).is_ok() {
                tracing::warn!("Identity user token used for admin-only endpoint");
                return Err(StatusCode::FORBIDDEN);
            }
            tracing::warn!("Invalid token for admin-only endpoint");
            return Err(StatusCode::UNAUTHORIZED);
        }
    };

    tracing::debug!(
        admin_user = %admin_claims.sub,
        tenant_id = %admin_claims.tenant_id,
        "Admin authenticated for management endpoint"
    );

    req.extensions_mut().insert(admin_claims);

    Ok(next.run(req).await)
}

// ============================================================================
// Shared helpers
// ============================================================================

/// Resolve auth context for admin principal, handling impersonation.
#[cfg(feature = "storage-rocksdb")]
async fn resolve_admin_auth_context(
    admin_claims: &raisin_rocksdb::AdminClaims,
    impersonate_user_id: Option<String>,
    uri_path: &str,
    state: &AppState,
) -> Result<raisin_models::auth::AuthContext, StatusCode> {
    use raisin_core::PermissionService;
    use raisin_models::auth::AuthContext;

    if let Some(target_user_id) = impersonate_user_id {
        if !admin_claims.access_flags.can_impersonate {
            tracing::warn!(
                admin_user = %admin_claims.sub,
                target_user = %target_user_id,
                "Impersonation denied - admin lacks can_impersonate flag"
            );
            return Err(StatusCode::FORBIDDEN);
        }

        let repo_id = extract_repo_from_path(uri_path).unwrap_or_else(|| "default".to_string());

        tracing::info!(
            admin_user = %admin_claims.sub,
            target_user = %target_user_id,
            repo_id = %repo_id,
            "Admin impersonating user"
        );

        let permission_service = PermissionService::new(state.storage().clone());
        let permissions = permission_service
            .resolve_for_user_id(&admin_claims.tenant_id, &repo_id, "main", &target_user_id)
            .await
            .map_err(|e| {
                tracing::error!(
                    target_user = %target_user_id,
                    error = %e,
                    "Failed to resolve permissions for impersonated user"
                );
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

        match permissions {
            Some(perms) => Ok(
                AuthContext::impersonated(&target_user_id, &admin_claims.sub)
                    .with_permissions(perms),
            ),
            None => {
                tracing::warn!(
                    target_user = %target_user_id,
                    "Impersonation target user not found"
                );
                Err(StatusCode::NOT_FOUND)
            }
        }
    } else {
        Ok(AuthContext::system())
    }
}

/// Resolve auth context for a regular user principal.
#[cfg(feature = "storage-rocksdb")]
async fn resolve_user_auth_context(
    user_claims: &raisin_models::auth::AuthClaims,
    uri_path: &str,
    state: &AppState,
) -> Result<raisin_models::auth::AuthContext, StatusCode> {
    use raisin_core::PermissionService;
    use raisin_models::auth::AuthContext;

    let repo_id = extract_repo_from_path(uri_path).unwrap_or_else(|| "default".to_string());

    let permission_service = PermissionService::new(state.storage().clone());
    let permissions = permission_service
        .resolve_for_identity_id(&user_claims.tenant_id, &repo_id, "main", &user_claims.sub)
        .await
        .map_err(|e| {
            tracing::error!(
                user = %user_claims.sub,
                error = %e,
                "Failed to resolve permissions for user"
            );
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let mut ctx = match permissions {
        Some(perms) => AuthContext::for_user(&user_claims.sub)
            .with_email(&user_claims.email)
            .with_permissions(perms),
        None => {
            tracing::debug!(
                user = %user_claims.sub,
                repo = %repo_id,
                "User has no explicit permissions for repository"
            );
            AuthContext::for_user(&user_claims.sub).with_email(&user_claims.email)
        }
    };
    if let Some(ref home) = user_claims.home {
        ctx = ctx.with_home(home);
    }
    Ok(ctx)
}

/// Check if anonymous access is enabled for a specific tenant/repo context.
///
/// Priority (highest to lowest):
/// 1. Repo-level config (node in raisin:system workspace)
/// 2. Tenant-level config (TenantAuthConfig in RocksDB)
/// 3. Global config (state.anonymous_enabled from server config)
#[cfg(feature = "storage-rocksdb")]
pub(super) async fn is_anonymous_enabled_for_context(
    storage: &std::sync::Arc<raisin_rocksdb::RocksDBStorage>,
    tenant_id: &str,
    repo_id: &str,
    global_anonymous_enabled: bool,
) -> bool {
    use raisin_core::services::node_service::NodeService;
    use raisin_models::auth::AuthContext;

    let repo_config_path = format!("/config/repos/{}", repo_id);

    let node_service: NodeService<raisin_rocksdb::RocksDBStorage> = NodeService::new_with_context(
        storage.clone(),
        tenant_id.to_string(),
        repo_id.to_string(),
        "main".to_string(),
        "raisin:system".to_string(),
    )
    .with_auth(AuthContext::system());

    let repo_config_node = node_service
        .get_by_path(&repo_config_path)
        .await
        .ok()
        .flatten();

    if let Some(node) = repo_config_node {
        if node.node_type == "raisin:RepoAuthConfig" {
            if let Some(raisin_models::nodes::properties::PropertyValue::Boolean(enabled)) =
                node.properties.get("anonymous_enabled")
            {
                tracing::debug!(
                    tenant_id = %tenant_id,
                    repo_id = %repo_id,
                    anonymous_enabled = %enabled,
                    "Anonymous access from repo config"
                );
                return *enabled;
            }
        }
    }

    let tenant_config = storage
        .tenant_auth_config_repository()
        .get_config(tenant_id)
        .await
        .ok()
        .flatten();

    if let Some(config) = tenant_config {
        tracing::debug!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            anonymous_enabled = %config.anonymous_enabled,
            "Anonymous access from tenant config"
        );
        return config.anonymous_enabled;
    }

    tracing::debug!(
        tenant_id = %tenant_id,
        repo_id = %repo_id,
        anonymous_enabled = %global_anonymous_enabled,
        "Anonymous access from global config (no tenant config found)"
    );
    global_anonymous_enabled
}
