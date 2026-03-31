// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Tenant authentication configuration handlers.

use axum::{
    extract::{Path, State},
    Json,
};

use crate::error::ApiError;
use crate::state::AppState;

use super::config_types::{TenantAuthConfigResponse, UpdateTenantAuthConfigRequest};

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
    Path(tenant_id): Path<String>,
) -> Result<Json<TenantAuthConfigResponse>, ApiError> {
    use raisin_models::auth::TenantAuthConfig;

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
    Path(tenant_id): Path<String>,
    Json(req): Json<UpdateTenantAuthConfigRequest>,
) -> Result<Json<TenantAuthConfigResponse>, ApiError> {
    use raisin_models::auth::{AuthProviderConfig, TenantAuthConfig};

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
