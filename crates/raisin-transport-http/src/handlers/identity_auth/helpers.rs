// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Shared helper functions for identity authentication handlers.

use axum::http::StatusCode;
use std::sync::Arc;

use crate::error::ApiError;
use crate::state::AppState;

#[cfg(feature = "storage-rocksdb")]
use raisin_models::auth::{AuthTokens, Identity, Session};
#[cfg(feature = "storage-rocksdb")]
use raisin_models::timestamp::StorageTimestamp;
#[cfg(feature = "storage-rocksdb")]
use raisin_rocksdb::repositories::{IdentityRepository, SessionRepository};
#[cfg(feature = "storage-rocksdb")]
use raisin_rocksdb::{AuthService, RocksDBStorage};

use super::constants::session_duration_nanos;
use super::policy::{load_auth_policy, EffectiveAuthPolicy};
use super::types::{AuthTokensResponse, IdentityInfo};

// ============================================================================
// Repository Access
// ============================================================================

/// Authentication repositories bundle for cleaner handler signatures.
#[cfg(feature = "storage-rocksdb")]
pub struct AuthRepositories {
    pub identity: IdentityRepository,
    pub session: SessionRepository,
    pub storage: Arc<RocksDBStorage>,
}

/// Extract authentication repositories from app state.
#[cfg(feature = "storage-rocksdb")]
pub fn extract_repos(state: &AppState) -> Result<AuthRepositories, ApiError> {
    let storage = state.rocksdb_storage.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "STORAGE_NOT_AVAILABLE",
            "RocksDB storage not configured",
        )
    })?;

    let db = storage.db().clone();
    let op = storage.operation_capture().clone();

    Ok(AuthRepositories {
        identity: IdentityRepository::new(db.clone(), op.clone()),
        session: SessionRepository::new(db, op),
        storage: storage.clone(),
    })
}

/// Get the AuthService from app state.
#[cfg(feature = "storage-rocksdb")]
pub fn get_auth_service(state: &AppState) -> Result<&Arc<AuthService>, ApiError> {
    state.auth_service.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "AUTH_SERVICE_NOT_AVAILABLE",
            "AuthService not configured",
        )
    })
}

// ============================================================================
// Session Creation
// ============================================================================

/// Create a new session for an identity.
#[cfg(feature = "storage-rocksdb")]
pub async fn create_session(
    session_repo: &SessionRepository,
    tenant_id: &str,
    identity_id: &str,
    auth_strategy: &str,
    remember_me: bool,
    actor: &str,
) -> Result<(Session, StorageTimestamp), ApiError> {
    use uuid::Uuid;

    let session_id = Uuid::new_v4().to_string();
    let token_family = Uuid::new_v4().to_string();

    let duration_nanos = session_duration_nanos(remember_me);
    let expires_at =
        StorageTimestamp::from_nanos(StorageTimestamp::now().timestamp_nanos() + duration_nanos)
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "TIMESTAMP_ERROR",
                    "Failed to compute session expiration",
                )
            })?;

    let session = Session::new(
        session_id,
        tenant_id.to_string(),
        identity_id.to_string(),
        auth_strategy.to_string(),
        token_family,
        expires_at,
    );

    session_repo
        .create(tenant_id, &session, actor)
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "DATABASE_ERROR",
                format!("Failed to create session: {}", e),
            )
        })?;

    Ok((session, expires_at))
}

// ============================================================================
// Token Generation
// ============================================================================

/// Generate authentication tokens for an identity.
///
/// This is the ONE token-minting path for every identity login (local
/// password, magic link, OpenID Connect): it loads the tenant's effective
/// auth policy and mints with the configured lifetimes. A handler that has
/// already loaded the policy can call [`generate_tokens_with_policy`]
/// instead and skip the second config read.
#[cfg(feature = "storage-rocksdb")]
pub async fn generate_tokens(
    state: &AppState,
    identity: &Identity,
    session: &Session,
    repo: Option<&str>,
    home: Option<&str>,
) -> Result<AuthTokens, ApiError> {
    let policy = load_auth_policy(state, &identity.tenant_id).await;
    generate_tokens_with_policy(state, identity, session, repo, home, &policy)
}

/// Generate authentication tokens with an already-loaded auth policy.
#[cfg(feature = "storage-rocksdb")]
pub fn generate_tokens_with_policy(
    state: &AppState,
    identity: &Identity,
    session: &Session,
    repo: Option<&str>,
    home: Option<&str>,
    policy: &EffectiveAuthPolicy,
) -> Result<AuthTokens, ApiError> {
    let auth_service = get_auth_service(state)?;

    auth_service
        .generate_user_tokens_with_lifetimes(
            identity,
            session,
            repo.map(String::from),
            home.map(String::from),
            policy.token_lifetimes(),
        )
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "TOKEN_GENERATION_ERROR",
                format!("Failed to generate tokens: {}", e),
            )
        })
}

// ============================================================================
// Response Building
// ============================================================================

/// Build an AuthTokensResponse from components.
///
/// `expires_at` is the ACCESS TOKEN expiry as a Unix timestamp in SECONDS,
/// the same unit the admin login and the refresh endpoint use. (It used to be
/// the session expiry in milliseconds, which disagreed with refresh.)
#[cfg(feature = "storage-rocksdb")]
pub fn build_auth_response(
    identity: &Identity,
    tokens: AuthTokens,
    home: Option<String>,
) -> AuthTokensResponse {
    let expires_at = access_token_expires_at(&tokens);
    AuthTokensResponse {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        token_type: "Bearer".to_string(),
        expires_at,
        identity: IdentityInfo::from_identity(identity, home),
    }
}

/// Unix timestamp (seconds) at which the access token in `tokens` expires.
#[cfg(feature = "storage-rocksdb")]
pub fn access_token_expires_at(tokens: &AuthTokens) -> i64 {
    chrono::Utc::now().timestamp() + i64::try_from(tokens.expires_in).unwrap_or(i64::MAX)
}

// ============================================================================
// Email Helpers
// ============================================================================

/// Mask an email address for display (e.g., "user@example.com" -> "us***@example.com")
pub fn mask_email(email: &str) -> String {
    if let Some(at_pos) = email.find('@') {
        let local = &email[..at_pos];
        let domain = &email[at_pos..];

        if local.len() < 2 {
            // Single char local: show it + mask
            format!("{}***{}", local, domain)
        } else {
            // 2+ char local: show first 2 chars + mask
            format!("{}***{}", &local[..2], domain)
        }
    } else {
        "***".to_string()
    }
}

// ============================================================================
// Validation Helpers
// ============================================================================

/// Validate email format.
pub fn validate_email(email: &str) -> Result<(), ApiError> {
    if !email.contains('@') {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "INVALID_EMAIL",
            "Invalid email format",
        ));
    }
    Ok(())
}

/// Validate password strength against the legacy fixed rule.
///
/// Handlers use `EffectiveAuthPolicy::validate_password`, which applies the
/// tenant's stored policy and falls back to this rule when none is stored.
pub fn validate_password(password: &str) -> Result<(), ApiError> {
    EffectiveAuthPolicy::default().validate_password(password)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "storage-rocksdb")]
    #[test]
    fn auth_response_expires_at_is_access_token_expiry_in_seconds() {
        let identity = Identity::new("id".into(), "t".into(), "a@example.com".into());
        let tokens = AuthTokens {
            access_token: "a".into(),
            refresh_token: "r".into(),
            token_type: "Bearer".into(),
            expires_in: 600,
            refresh_expires_in: Some(7200),
        };
        let before = chrono::Utc::now().timestamp();
        let resp = build_auth_response(&identity, tokens, None);
        let after = chrono::Utc::now().timestamp();
        assert!(resp.expires_at >= before + 600 && resp.expires_at <= after + 600);
        // Seconds, never milliseconds: a millisecond epoch is > 1e11.
        assert!(resp.expires_at < 100_000_000_000);
    }

    #[test]
    fn test_mask_email() {
        assert_eq!(mask_email("user@example.com"), "us***@example.com");
        assert_eq!(mask_email("a@example.com"), "a***@example.com");
        assert_eq!(mask_email("ab@example.com"), "ab***@example.com");
        assert_eq!(mask_email("invalid"), "***");
    }
}
