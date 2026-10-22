// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Self-service password change for identity users.
//!
//! Distinct from `handlers::auth::change_password`, which serves the admin
//! console user store (`AdminUser`). This one operates on `Identity` /
//! `LocalCredentials`, so an application user who was provisioned with
//! `must_change_password` can actually clear that flag themselves — nothing
//! else in the system can do it on their behalf without an operator resetting
//! the password outright.

use axum::{http::StatusCode, Extension, Json};
use raisin_models::auth::AuthClaims;
use serde::Deserialize;

use crate::error::ApiError;
use crate::state::AppState;

use super::policy::load_auth_policy;

/// How long after redeeming a reset link the session may set a password without
/// the old one. The refresh token can outlive this, so it is measured from the
/// SESSION's creation, not from the access token (whose `auth_time` is re-stamped
/// on every refresh).
const RESET_WINDOW_SECS: i64 = 15 * 60;

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    /// Required, except on a session minted by a password-reset link — whose
    /// whole point is that the user no longer knows it.
    #[serde(default)]
    pub old_password: Option<String>,
    pub new_password: String,
}

/// Change the calling identity's own password.
///
/// # Endpoint
/// POST /auth/change-password
/// POST /auth/{repo}/change-password
///
/// # Headers
/// Authorization: Bearer {access_token}
///
/// Verifies `old_password` (omitted only on a fresh password-reset session, see
/// [`RESET_WINDOW_SECS`]), then replaces the stored credentials — which
/// clears `must_change_password` and any lockout counters. Returns 204.
///
/// The tenant and identity both come from the token, never the request body,
/// so this cannot be pointed at another account.
#[cfg(feature = "storage-rocksdb")]
pub async fn change_password(
    axum::extract::State(state): axum::extract::State<AppState>,
    claims: Option<Extension<AuthClaims>>,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<StatusCode, ApiError> {
    use raisin_rocksdb::repositories::IdentityRepository;

    // An admin-console token authenticates but is not an identity — it has no
    // LocalCredentials to change. Say so rather than 500ing on a missing
    // extension.
    let Some(Extension(claims)) = claims else {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "IDENTITY_TOKEN_REQUIRED",
            "This endpoint requires an identity access token",
        ));
    };

    load_auth_policy(&state, &claims.tenant_id)
        .await
        .validate_password(&req.new_password)?;

    if req.old_password.as_deref() == Some(req.new_password.as_str()) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "PASSWORD_UNCHANGED",
            "New password must differ from the current password",
        ));
    }

    let rocksdb_storage = state.rocksdb_storage.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "STORAGE_NOT_AVAILABLE",
            "RocksDB storage not available",
        )
    })?;

    let identity_repo = IdentityRepository::new(
        rocksdb_storage.db().clone(),
        rocksdb_storage.operation_capture().clone(),
    );

    match req.old_password.as_deref() {
        Some(old) => {
            let matches = identity_repo
                .verify_password(&claims.tenant_id, &claims.sub, old)
                .await
                .map_err(ApiError::from)?;
            if !matches {
                return Err(ApiError::new(
                    StatusCode::UNAUTHORIZED,
                    "INVALID_CREDENTIALS",
                    "Current password is incorrect",
                ));
            }
        }
        None => {
            // No old password: only a live, recent password-reset session may
            // do this. Read from the stored session, never from the token — a
            // claim is a copy and this is the authority.
            let session_repo = raisin_rocksdb::repositories::SessionRepository::new(
                rocksdb_storage.db().clone(),
                rocksdb_storage.operation_capture().clone(),
            );
            let session = session_repo
                .get(&claims.tenant_id, &claims.sid)
                .await
                .map_err(ApiError::from)?;
            let age_secs = session.as_ref().map(|s| {
                (raisin_models::timestamp::StorageTimestamp::now().timestamp_nanos()
                    - s.created_at.timestamp_nanos())
                    / 1_000_000_000
            });
            let allowed = matches!(
                (&session, age_secs),
                (Some(s), Some(age))
                    if !s.revoked
                        && s.strategy_id == super::magic_link::RESET_STRATEGY
                        && s.identity_id == claims.sub
                        && (0..=RESET_WINDOW_SECS).contains(&age)
            );
            if !allowed {
                return Err(ApiError::new(
                    StatusCode::UNAUTHORIZED,
                    "OLD_PASSWORD_REQUIRED",
                    "The current password is required, or request a password reset link",
                ));
            }
        }
    }

    identity_repo
        .set_password(
            &claims.tenant_id,
            &claims.sub,
            &req.new_password,
            false,
            "self:change-password",
        )
        .await
        .map_err(ApiError::from)?;

    tracing::info!(
        tenant_id = %claims.tenant_id,
        identity_id = %claims.sub,
        "Identity changed its own password"
    );

    Ok(StatusCode::NO_CONTENT)
}
