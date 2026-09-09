// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Loading a tenant's OIDC provider and building a strategy for it.

use axum::http::StatusCode;
use raisin_auth::strategies::OidcStrategy;
use raisin_auth::AuthStrategy;
use raisin_models::auth::{AuthProviderConfig, TenantAuthConfig};

use crate::error::ApiError;
use crate::state::AppState;

#[cfg(feature = "storage-rocksdb")]
pub(super) async fn load_tenant_config(
    state: &AppState,
    tenant_id: &str,
) -> Result<TenantAuthConfig, ApiError> {
    state
        .storage()
        .tenant_auth_config_repository()
        .get_config(tenant_id)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to load the tenant auth config: {e}")))?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "NO_AUTH_CONFIG",
                "This tenant has no authentication configuration, so no OIDC provider is set up",
            )
        })
}

/// Find an enabled OIDC provider and initialise a strategy for it.
///
/// Built per request rather than held in a registry, so an administrator's
/// change to a provider takes effect on the next login instead of the next
/// restart. Discovery and JWKS are cached inside raisin-auth, so the cost of
/// rebuilding is a map lookup, not an HTTP round trip.
#[cfg(feature = "storage-rocksdb")]
pub(super) async fn build_strategy(
    state: &AppState,
    tenant_id: &str,
    config: &TenantAuthConfig,
    provider: &str,
) -> Result<OidcStrategy, ApiError> {
    let strategy_id = format!("oidc:{provider}");
    let provider_config: &AuthProviderConfig = config
        .providers
        .iter()
        .find(|p| p.strategy_id == strategy_id)
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "UNKNOWN_PROVIDER",
                format!("No OIDC provider '{provider}' is configured for this tenant"),
            )
        })?;

    if !provider_config.enabled {
        // Distinguished from "unknown" on purpose: an administrator who
        // disabled a provider should see that, not a 404 suggesting a typo.
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "PROVIDER_DISABLED",
            format!("The OIDC provider '{provider}' is disabled"),
        ));
    }

    let client_secret = match &provider_config.client_secret_encrypted {
        Some(sealed) if !sealed.is_empty() => {
            super::super::open_client_secret(&master_key(state)?, tenant_id, sealed)?
        }
        // No stored secret means a public client, which PKCE alone protects.
        _ => String::new(),
    };

    let mut strategy = OidcStrategy::new(provider, provider_config.display_name.clone());
    strategy
        .init(provider_config, Some(&client_secret))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "OIDC_PROVIDER_UNAVAILABLE",
                format!("Could not initialise the OIDC provider '{provider}': {e}"),
            )
        })?;

    Ok(strategy)
}

pub(super) fn master_key(state: &AppState) -> Result<[u8; 32], ApiError> {
    state.get_master_key().map_err(|_| {
        ApiError::internal(
            "RAISIN_MASTER_KEY is not configured, so OIDC login state cannot be sealed",
        )
    })
}
