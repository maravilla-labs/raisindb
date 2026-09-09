// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Token refresh handler. Logout, listing and revocation live in `sessions.rs`.

use axum::{extract::State, http::StatusCode, Json};

use crate::error::ApiError;
use crate::state::AppState;

use super::helpers::{extract_repos, get_auth_service};
use super::types::{AuthTokensResponse, IdentityInfo, RefreshTokenRequest};

/// Refresh authentication tokens.
///
/// # Endpoint
/// POST /auth/refresh
///
/// # Security
/// This endpoint implements token rotation for security:
/// - Each refresh invalidates the old refresh token
/// - Generation counter tracks token versions
/// - Token reuse triggers session revocation (possible attack detection)
/// - All operations are replicated for cluster consistency
#[cfg(feature = "storage-rocksdb")]
pub async fn refresh_token(
    State(state): State<AppState>,
    Json(req): Json<RefreshTokenRequest>,
) -> Result<Json<AuthTokensResponse>, ApiError> {
    let repos = extract_repos(&state)?;
    let auth_service = get_auth_service(&state)?;

    // 1. Validate refresh token (signature, expiration)
    let refresh_claims = auth_service
        .validate_refresh_token(&req.refresh_token)
        .map_err(|e| {
            tracing::warn!("Invalid refresh token: {}", e);
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "INVALID_REFRESH_TOKEN",
                "Invalid or expired refresh token",
            )
        })?;

    let tenant_id = &refresh_claims.tenant_id;
    let session_id = &refresh_claims.sid;

    // 2. Get session from storage
    let session = repos
        .session
        .get(tenant_id, session_id)
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "DATABASE_ERROR",
                format!("Failed to retrieve session: {}", e),
            )
        })?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "SESSION_NOT_FOUND",
                "Session not found or has been revoked",
            )
        })?;

    // 3. Verify session is still valid (not revoked, not expired)
    if session.revoked {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "SESSION_REVOKED",
            "Session has been revoked",
        ));
    }

    if session.is_expired() {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "SESSION_EXPIRED",
            "Session has expired",
        ));
    }

    // 4. Verify token family and generation (detect token reuse attacks)
    if session.token_family != refresh_claims.family {
        tracing::warn!(
            "Token family mismatch for session {} - possible attack",
            session_id
        );
        // Revoke the session as a security measure
        let _ = repos
            .session
            .revoke(tenant_id, session_id, "token_family_mismatch", "system")
            .await;
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "TOKEN_FAMILY_MISMATCH",
            "Token family mismatch - session has been revoked for security",
        ));
    }

    if session.token_generation != refresh_claims.generation {
        tracing::warn!(
            "Token generation mismatch for session {} (expected {}, got {}) - possible token reuse",
            session_id,
            session.token_generation,
            refresh_claims.generation
        );
        // Revoke the session - this is a potential token reuse attack
        let _ = repos
            .session
            .revoke(tenant_id, session_id, "token_reuse_detected", "system")
            .await;
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "TOKEN_REUSE_DETECTED",
            "Token reuse detected - session has been revoked for security",
        ));
    }

    // 5. Get identity
    let identity = repos
        .identity
        .get(tenant_id, &refresh_claims.sub)
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "DATABASE_ERROR",
                format!("Failed to retrieve identity: {}", e),
            )
        })?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "IDENTITY_NOT_FOUND",
                "Identity not found",
            )
        })?;

    // Check if identity is still active
    if !identity.is_active {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "ACCOUNT_DISABLED",
            "This account has been disabled",
        ));
    }

    // 6. Generate new tokens with incremented generation (preserves home from refresh token)
    let policy = super::policy::load_auth_policy(&state, tenant_id).await;
    let (tokens, new_generation) = auth_service
        .refresh_user_tokens_with_lifetimes(
            &identity,
            &session,
            &refresh_claims,
            refresh_claims.home.clone(),
            policy.token_lifetimes(),
        )
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "TOKEN_GENERATION_ERROR",
                format!("Failed to generate tokens: {}", e),
            )
        })?;

    // 7. Rotate refresh token in storage (with cluster replication)
    repos
        .session
        .rotate_refresh_token(
            tenant_id,
            session_id,
            new_generation,
            "system:token_refresh",
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to rotate refresh token: {}", e);
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "DATABASE_ERROR",
                "Failed to update session",
            )
        })?;

    tracing::info!(
        identity_id = %identity.identity_id,
        session_id = %session_id,
        new_generation = %new_generation,
        "Token refreshed successfully"
    );

    // 8. Return new tokens (home is preserved from original refresh token)
    let expires_at = super::helpers::access_token_expires_at(&tokens);
    Ok(Json(AuthTokensResponse {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        token_type: "Bearer".to_string(),
        expires_at,
        identity: IdentityInfo::from_identity(&identity, refresh_claims.home),
    }))
}
