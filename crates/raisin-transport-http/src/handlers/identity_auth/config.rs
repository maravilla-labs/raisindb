// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Tenant authentication configuration handlers.

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use raisin_models::auth::AuthContext;

use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::AppState;

use super::config_types::{TenantAuthConfigResponse, UpdateTenantAuthConfigRequest};

/// Reject the request unless the caller is an admin **of the tenant they named**.
///
/// Both halves are load-bearing, and neither existed before: these two handlers
/// took `State, Path, Json` and nothing else, their route carried no
/// `require_auth_middleware` (unlike every neighbour), and the router has no
/// blanket auth layer. `PUT /api/tenants/{id}/auth/config` was therefore
/// writable by anyone who could reach the port — and it sets `default_roles`
/// for every new signup, `anonymous_enabled`, the password policy and the CORS
/// origins. `default_roles: ["admin"]` followed by a registration is a
/// straight path to admin.
///
/// The TENANT BINDING is the second half. `tenant_id` arrives as a PATH
/// parameter while the real tenant is resolved upstream (from the host, by the
/// proxy, into `TenantInfo`). Trusting the path would let an admin of one
/// tenant rewrite another's auth config by typing a different id, so the path
/// is treated as an assertion to be checked, never as a selector.
fn require_tenant_admin(
    auth: Option<&AuthContext>,
    tenant_info: &TenantInfo,
    tenant_id: &str,
) -> Result<(), ApiError> {
    let is_admin = auth.is_some_and(|a| {
        a.is_system
            || a.roles.iter().any(|r| r == "system_admin" || r == "admin")
            || a.permissions().is_some_and(|p| p.is_system_admin)
    });
    if !is_admin {
        return Err(ApiError::new(
            axum::http::StatusCode::FORBIDDEN,
            "FORBIDDEN",
            "Tenant authentication configuration requires an admin principal",
        ));
    }

    // A system principal may act across tenants; anyone else is confined to the
    // tenant this request actually resolved to.
    let cross_tenant = tenant_id != tenant_info.tenant_id;
    let is_system = auth.is_some_and(|a| a.is_system);
    if cross_tenant && !is_system {
        tracing::warn!(
            path_tenant = %tenant_id,
            resolved_tenant = %tenant_info.tenant_id,
            "refused a cross-tenant auth-config request"
        );
        return Err(ApiError::new(
            axum::http::StatusCode::FORBIDDEN,
            "FORBIDDEN",
            "Tenant authentication configuration may only be read or written for the \
             tenant this request resolved to",
        ));
    }
    Ok(())
}

/// Get tenant authentication configuration.
///
/// # Endpoint
/// GET /api/tenants/{tenant_id}/auth/config
///
/// Returns the tenant's authentication configuration including providers,
/// session settings, password policy, and anonymous access settings.
#[cfg(feature = "storage-rocksdb")]
pub async fn get_auth_config(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    Path(tenant_id): Path<String>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<TenantAuthConfigResponse>, ApiError> {
    use raisin_models::auth::TenantAuthConfig;

    // The READ is gated too: it discloses the password policy, which providers
    // are enabled, the CORS origins and the default roles — a map of what to
    // attack, and none of an unauthenticated caller's business.
    require_tenant_admin(auth.as_deref(), &tenant_info, &tenant_id)?;

    let storage = state.storage();

    // Get tenant auth config from storage
    let config = storage
        .tenant_auth_config_repository()
        .get_config(&tenant_id)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to load auth config: {}", e)))?
        .unwrap_or_else(|| TenantAuthConfig::new(tenant_id.clone()));

    // Convert to API response format
    let response = TenantAuthConfigResponse::from_config(&config);

    Ok(Json(response))
}

/// Update tenant authentication configuration.
///
/// # Endpoint
/// PUT /api/tenants/{tenant_id}/auth/config
///
/// Updates the tenant's authentication configuration.
#[cfg(feature = "storage-rocksdb")]
pub async fn update_auth_config(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    Path(tenant_id): Path<String>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<UpdateTenantAuthConfigRequest>,
) -> Result<Json<TenantAuthConfigResponse>, ApiError> {
    use raisin_models::auth::{AuthProviderConfig, TenantAuthConfig};

    require_tenant_admin(auth.as_deref(), &tenant_info, &tenant_id)?;

    let storage = state.storage();

    // Get existing config or create new one
    let mut config = storage
        .tenant_auth_config_repository()
        .get_config(&tenant_id)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to load auth config: {}", e)))?
        .unwrap_or_else(|| TenantAuthConfig::new(tenant_id.clone()));

    // Update local auth settings
    if let Some(local_auth) = &req.local_auth {
        // Find or create local auth provider
        let local_provider_exists = config.providers.iter().any(|p| p.strategy_id == "local");
        if !local_provider_exists && local_auth.enabled {
            config.providers.push(AuthProviderConfig::local());
        }
        for provider in &mut config.providers {
            if provider.strategy_id == "local" {
                provider.enabled = local_auth.enabled;
            }
        }
    }

    // Update magic link settings
    if let Some(magic_link) = &req.magic_link {
        // Find or create magic link provider
        let magic_link_exists = config
            .providers
            .iter()
            .any(|p| p.strategy_id == "magic_link");
        if !magic_link_exists && magic_link.enabled {
            config.providers.push(AuthProviderConfig::magic_link());
        }
        for provider in &mut config.providers {
            if provider.strategy_id == "magic_link" {
                provider.enabled = magic_link.enabled;
            }
        }
    }

    // Update password policy
    if let Some(policy) = &req.password_policy {
        config.password_policy.min_length = policy.min_length;
        config.password_policy.require_uppercase = policy.require_uppercase;
        config.password_policy.require_lowercase = policy.require_lowercase;
        config.password_policy.require_digit = policy.require_numbers;
        config.password_policy.require_special = policy.require_special;
        config.password_policy.expiry_days = policy.max_age_days.unwrap_or(0);
    }

    // Update session settings
    if let Some(session) = &req.session_settings {
        config.session_settings.access_token_duration_seconds =
            (session.duration_hours as u64) * 3600;
        config.session_settings.refresh_token_duration_seconds =
            (session.refresh_token_duration_days as u64) * 24 * 3600;
        config.session_settings.max_sessions_per_user = if session.single_session_mode {
            1
        } else {
            session.max_sessions_per_user
        };
    }

    // Update access settings
    if let Some(access) = &req.access_settings {
        config.access_settings.allow_access_requests = access.allow_access_requests;
        config.access_settings.allow_invitations = access.allow_invitations;
        config.access_settings.require_approval = access.require_approval;
        config.access_settings.default_roles = access.default_roles.clone();
    }

    // Update anonymous access setting
    if let Some(anonymous_enabled) = req.anonymous_enabled {
        config.anonymous_enabled = anonymous_enabled;
    }

    // Update CORS allowed origins
    if let Some(cors_origins) = req.cors_allowed_origins {
        config.cors_allowed_origins = cors_origins;
    }

    // Save the updated config
    storage
        .tenant_auth_config_repository()
        .set_config(&config)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to save auth config: {}", e)))?;

    tracing::info!(
        tenant_id = %tenant_id,
        anonymous_enabled = config.anonymous_enabled,
        "Updated tenant auth config"
    );

    // Return updated config
    let response = TenantAuthConfigResponse::from_config(&config);

    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tenant(id: &str) -> TenantInfo {
        TenantInfo {
            tenant_id: id.to_string(),
            deployment_key: "test".to_string(),
        }
    }

    fn admin() -> AuthContext {
        let mut ctx = AuthContext::anonymous();
        ctx.is_anonymous = false;
        ctx.user_id = Some("u1".to_string());
        ctx.roles = vec!["admin".to_string()];
        ctx
    }

    /// The regression this exists for. These handlers took no `AuthContext` and
    /// their route carried no auth layer, so `PUT /api/tenants/{id}/auth/config`
    /// — which sets `default_roles` for every new signup, `anonymous_enabled`
    /// and the CORS origins — was writable by anyone who could reach the port.
    #[test]
    fn an_unauthenticated_caller_is_refused() {
        let err = require_tenant_admin(None, &tenant("acme"), "acme")
            .expect_err("no principal must be refused");
        assert_eq!(err.status, axum::http::StatusCode::FORBIDDEN);
    }

    /// Being signed in is not enough: this is admin configuration.
    #[test]
    fn a_non_admin_principal_is_refused() {
        let mut viewer = admin();
        viewer.roles = vec!["viewer".to_string()];
        assert!(require_tenant_admin(Some(&viewer), &tenant("acme"), "acme").is_err());
    }

    /// The tenant comes from the request, not from the URL. An admin of one
    /// tenant must not be able to rewrite another's auth config by typing a
    /// different id into the path.
    #[test]
    fn an_admin_cannot_reach_another_tenant_through_the_path() {
        let err = require_tenant_admin(Some(&admin()), &tenant("acme"), "othercorp")
            .expect_err("a mismatched path tenant must be refused");
        assert_eq!(err.status, axum::http::StatusCode::FORBIDDEN);
    }

    /// The complement, so the tests above would fail if this denied everything.
    #[test]
    fn an_admin_of_the_resolved_tenant_is_allowed() {
        assert!(require_tenant_admin(Some(&admin()), &tenant("acme"), "acme").is_ok());
    }

    /// A system principal legitimately acts across tenants (bootstrap, the
    /// admin console's own operator surface), so the binding does not apply.
    #[test]
    fn a_system_principal_may_cross_tenants() {
        let sys = AuthContext::system();
        assert!(require_tenant_admin(Some(&sys), &tenant("acme"), "othercorp").is_ok());
    }
}
