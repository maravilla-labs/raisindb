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
#[cfg(feature = "storage-rocksdb")]
pub fn generate_tokens(
    state: &AppState,
    identity: &Identity,
    session: &Session,
    repo: Option<&str>,
    home: Option<&str>,
) -> Result<AuthTokens, ApiError> {
    let auth_service = get_auth_service(state)?;

    auth_service
        .generate_user_tokens(
            identity,
            session,
            repo.map(String::from),
            home.map(String::from),
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
#[cfg(feature = "storage-rocksdb")]
pub fn build_auth_response(
    identity: &Identity,
    tokens: AuthTokens,
    expires_at: StorageTimestamp,
    home: Option<String>,
) -> AuthTokensResponse {
    AuthTokensResponse {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        token_type: "Bearer".to_string(),
        expires_at: expires_at.timestamp_nanos() / 1_000_000, // Convert to millis
        identity: IdentityInfo::from_identity(identity, home),
    }
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

/// Validate password strength.
pub fn validate_password(password: &str) -> Result<(), ApiError> {
    if password.len() < 8 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "WEAK_PASSWORD",
            "Password must be at least 8 characters long",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mask_email() {
        assert_eq!(mask_email("user@example.com"), "us***@example.com");
        assert_eq!(mask_email("a@example.com"), "a***@example.com");
        assert_eq!(mask_email("ab@example.com"), "ab***@example.com");
        assert_eq!(mask_email("invalid"), "***");
    }
}
