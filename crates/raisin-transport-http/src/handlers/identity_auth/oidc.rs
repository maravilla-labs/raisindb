// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! OIDC (OpenID Connect) authentication handlers.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Redirect,
    Extension, Json,
};

use crate::error::ApiError;
use crate::state::AppState;

use super::types::{AuthTokensResponse, OidcAuthQuery, OidcCallbackQuery};

/// Start OIDC authentication flow.
///
/// # Endpoint
/// GET /auth/oidc/{provider}
///
/// This redirects to the OIDC provider's authorization endpoint.
#[cfg(feature = "storage-rocksdb")]
pub async fn oidc_authorize(
    State(_state): State<AppState>,
    Extension(tenant_info): Extension<crate::middleware::TenantInfo>,
    Path(provider): Path<String>,
    Query(query): Query<OidcAuthQuery>,
) -> Result<Redirect, ApiError> {
    // TODO: Implement via IdentityAuthService + OidcStrategy
    // 1. Load provider config for tenant
    // 2. Generate state and PKCE verifier
    // 3. Store state in session/cookie
    // 4. Build authorization URL
    // 5. Redirect to provider

    Err(ApiError::new(
        StatusCode::NOT_IMPLEMENTED,
        "NOT_IMPLEMENTED",
        format!("OIDC provider '{}' not yet implemented", provider),
    ))
}

/// Handle OIDC callback.
///
/// # Endpoint
/// GET /auth/oidc/{provider}/callback
///
/// This handles the callback from the OIDC provider after authorization.
#[cfg(feature = "storage-rocksdb")]
pub async fn oidc_callback(
    State(_state): State<AppState>,
    Extension(tenant_info): Extension<crate::middleware::TenantInfo>,
    Path(provider): Path<String>,
    Query(query): Query<OidcCallbackQuery>,
) -> Result<Json<AuthTokensResponse>, ApiError> {
    // Check for error from provider
    if let Some(error) = &query.error {
        let message = query
            .error_description
            .as_deref()
            .unwrap_or("Authorization failed");
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, error, message));
    }

    // TODO: Implement via IdentityAuthService + OidcStrategy
    // 1. Verify state parameter
    // 2. Exchange code for tokens
    // 3. Validate ID token
    // 4. Extract user info
    // 5. Find or create identity (with linking)
    // 6. Create session
    // 7. Generate tokens

    Err(ApiError::new(
        StatusCode::NOT_IMPLEMENTED,
        "NOT_IMPLEMENTED",
        format!("OIDC callback for '{}' not yet implemented", provider),
    ))
}
