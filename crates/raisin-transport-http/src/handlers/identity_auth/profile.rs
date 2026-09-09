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

use super::types::{AuthProviderInfo, AuthProvidersResponse, MeForRepoResponse, MeResponse};

/// Build the provider list from the tenant's stored auth config.
///
/// Local and magic-link sign-in default to ENABLED when the tenant has never
/// written a config: their login routes do not consult the config, so
/// reporting them off would make a UI hide a button that works. An explicit
/// `enabled: false` entry is honoured. OIDC providers are listed only when
/// enabled, with the server-relative URL that starts a login; `repo` is
/// appended so the callback provisions the user into that repository.
#[cfg(feature = "storage-rocksdb")]
async fn providers_response(
    state: &AppState,
    tenant_id: &str,
    repo: Option<&str>,
) -> Result<AuthProvidersResponse, ApiError> {
    use raisin_models::auth::TenantAuthConfig;

    let config = state
        .storage()
        .tenant_auth_config_repository()
        .get_config(tenant_id)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to load auth config: {e}")))?
        .unwrap_or_else(|| TenantAuthConfig::new(tenant_id.to_string()));

    Ok(providers_from_config(&config, repo))
}

/// The pure half of [`providers_response`], separated so it can be tested
/// without storage.
pub(super) fn providers_from_config(
    config: &raisin_models::auth::TenantAuthConfig,
    repo: Option<&str>,
) -> AuthProvidersResponse {
    let enabled_or_default = |strategy: &str| {
        config
            .get_provider(strategy)
            .map(|p| p.enabled)
            .unwrap_or(true)
    };

    let providers = super::config_oidc::oidc_provider_views(config)
        .into_iter()
        .filter(|p| p.enabled)
        .map(|p| {
            let auth_url = match repo {
                Some(r) => format!("{}?repo={}", p.authorize_url, urlencoding::encode(r)),
                None => p.authorize_url.clone(),
            };
            AuthProviderInfo {
                id: p.provider_id,
                display_name: p.display_name,
                icon: p.icon,
                auth_url,
            }
        })
        .collect();

    AuthProvidersResponse {
        providers,
        local_enabled: enabled_or_default("local"),
        magic_link_enabled: enabled_or_default("magic_link"),
    }
}

/// Get available authentication providers for a tenant.
///
/// # Endpoint
/// GET /auth/providers
///
/// Returns the sign-in methods a login page should offer: whether password and
/// magic-link sign-in are on, and every enabled OIDC provider with the URL
/// that starts its flow. No secrets, no client ids.
#[cfg(feature = "storage-rocksdb")]
pub async fn get_providers(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<crate::middleware::TenantInfo>,
) -> Result<Json<AuthProvidersResponse>, ApiError> {
    Ok(Json(
        providers_response(&state, &tenant_info.tenant_id, None).await?,
    ))
}

/// Get available authentication providers for a specific repository.
///
/// # Endpoint
/// GET /auth/{repo}/providers
///
/// Same list as `/auth/providers`; the OIDC start URLs carry `?repo=` so the
/// callback provisions the user node in this repository.
#[cfg(feature = "storage-rocksdb")]
pub async fn get_providers_for_repo(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<crate::middleware::TenantInfo>,
    Path(repo): Path<String>,
) -> Result<Json<AuthProvidersResponse>, ApiError> {
    Ok(Json(
        providers_response(&state, &tenant_info.tenant_id, Some(&repo)).await?,
    ))
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

#[cfg(all(test, feature = "storage-rocksdb"))]
mod tests {
    use super::*;
    use raisin_models::auth::{AuthProviderConfig, TenantAuthConfig};

    /// A tenant that never saved a config still gets working password and
    /// magic-link buttons: those routes do not read the config.
    #[test]
    fn an_empty_config_reports_local_and_magic_link_on() {
        let r = providers_from_config(&TenantAuthConfig::new("t".into()), None);
        assert!(r.local_enabled && r.magic_link_enabled);
        assert!(r.providers.is_empty());
    }

    #[test]
    fn an_explicit_disable_is_honoured() {
        let mut cfg = TenantAuthConfig::new("t".into());
        let mut local = AuthProviderConfig::local();
        local.enabled = false;
        cfg.providers.push(local);
        assert!(!providers_from_config(&cfg, None).local_enabled);
    }

    #[test]
    fn enabled_oidc_providers_are_listed_with_their_start_url() {
        let mut cfg = TenantAuthConfig::new("t".into());
        cfg.providers
            .push(AuthProviderConfig::oidc("keycloak", "Staff SSO"));
        let mut off = AuthProviderConfig::oidc("legacy", "Old");
        off.enabled = false;
        cfg.providers.push(off);

        let r = providers_from_config(&cfg, Some("my repo"));
        assert_eq!(r.providers.len(), 1);
        assert_eq!(r.providers[0].id, "keycloak");
        assert_eq!(r.providers[0].display_name, "Staff SSO");
        assert_eq!(
            r.providers[0].auth_url,
            "/auth/oidc/keycloak?repo=my%20repo"
        );
        assert_eq!(
            providers_from_config(&cfg, None).providers[0].auth_url,
            "/auth/oidc/keycloak"
        );
    }
}
