// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Session management: logout, list, revoke one.
//!
//! All three act on the SESSION record behind a refresh token, which is what
//! `POST /auth/refresh` consults. Revoking a session stops the next refresh;
//! the access token already issued stays valid until its own expiry, because
//! access tokens are verified statelessly. That is the trade the short access
//! lifetime buys, and it is stated in the docs.
//!
//! The caller is identified by re-validating the bearer token here rather than
//! from `AuthContext`, because the context carries no session id and these
//! endpoints need the `sid` claim to know which session is "current".

use axum::{
    extract::{Path, State},
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    Json,
};

use crate::error::ApiError;
use crate::state::AppState;

use super::types::{SessionInfo, SessionsResponse};

#[cfg(feature = "storage-rocksdb")]
use super::helpers::{extract_repos, get_auth_service};

/// The identity and session a bearer access token names.
#[cfg(feature = "storage-rocksdb")]
fn caller(state: &AppState, headers: &HeaderMap) -> Result<(String, String, String), ApiError> {
    let unauthorized = || {
        ApiError::new(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "A valid identity access token is required",
        )
    };
    let token = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(unauthorized)?;

    // Admin-console and API-key tokens have no session; only an identity
    // access token does, so only that kind is accepted here.
    let claims = get_auth_service(state)?
        .validate_user_token(token)
        .map_err(|_| unauthorized())?;
    Ok((claims.tenant_id, claims.sub, claims.sid))
}

fn to_info(session: &raisin_models::auth::Session, current_sid: &str) -> SessionInfo {
    SessionInfo {
        id: session.session_id.clone(),
        auth_strategy: session.strategy_id.clone(),
        user_agent: session.client_info.user_agent.clone(),
        ip_address: session.client_info.ip_address.clone(),
        created_at: session.created_at.as_datetime().to_rfc3339(),
        last_active_at: session.last_activity_at.as_datetime().to_rfc3339(),
        is_current: session.session_id == current_sid,
    }
}

/// Logout and revoke the current session.
///
/// # Endpoint
/// POST /auth/logout
///
/// # Headers
/// Authorization: Bearer {access_token}
///
/// Answers `204` whether or not the session still existed, so a second logout
/// is harmless.
#[cfg(feature = "storage-rocksdb")]
pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let (tenant_id, identity_id, sid) = caller(&state, &headers)?;
    let repos = extract_repos(&state)?;

    repos
        .session
        .revoke(
            &tenant_id,
            &sid,
            "logout",
            &format!("identity:{identity_id}"),
        )
        .await
        .map_err(|e| ApiError::internal(format!("Failed to revoke session: {e}")))?;

    tracing::info!(identity_id = %identity_id, session_id = %sid, "Session revoked by logout");
    Ok(StatusCode::NO_CONTENT)
}

/// List the caller's live sessions.
///
/// # Endpoint
/// GET /auth/sessions
///
/// Revoked and expired sessions are omitted; the one the request was made
/// with is flagged `is_current`.
#[cfg(feature = "storage-rocksdb")]
pub async fn list_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<SessionsResponse>, ApiError> {
    let (tenant_id, identity_id, sid) = caller(&state, &headers)?;
    let repos = extract_repos(&state)?;

    let mut sessions: Vec<SessionInfo> = repos
        .session
        .list_for_identity(&tenant_id, &identity_id)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to list sessions: {e}")))?
        .iter()
        .filter(|s| s.is_valid())
        .map(|s| to_info(s, &sid))
        .collect();
    sessions.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    Ok(Json(SessionsResponse { sessions }))
}

/// Revoke one of the caller's sessions.
///
/// # Endpoint
/// DELETE /auth/sessions/{session_id}
///
/// A session id belonging to another identity answers `404`, the same as an
/// unknown id, so the endpoint cannot be used to probe for other users'
/// sessions.
#[cfg(feature = "storage-rocksdb")]
pub async fn revoke_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let (tenant_id, identity_id, _sid) = caller(&state, &headers)?;
    let repos = extract_repos(&state)?;

    let owned = repos
        .session
        .get(&tenant_id, &session_id)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to load session: {e}")))?
        .filter(|s| s.identity_id == identity_id);
    if owned.is_none() {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "SESSION_NOT_FOUND",
            "No such session",
        ));
    }

    repos
        .session
        .revoke(
            &tenant_id,
            &session_id,
            "revoked_by_user",
            &format!("identity:{identity_id}"),
        )
        .await
        .map_err(|e| ApiError::internal(format!("Failed to revoke session: {e}")))?;

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::auth::Session;
    use raisin_models::timestamp::StorageTimestamp;

    fn session(id: &str) -> Session {
        Session::new(
            id.to_string(),
            "t".to_string(),
            "me".to_string(),
            "oidc:keycloak".to_string(),
            "fam".to_string(),
            StorageTimestamp::from_nanos(StorageTimestamp::now().timestamp_nanos() + 1_000_000_000)
                .unwrap(),
        )
    }

    #[test]
    fn the_calling_session_is_flagged_current() {
        assert!(to_info(&session("a"), "a").is_current);
        assert!(!to_info(&session("b"), "a").is_current);
        assert_eq!(to_info(&session("a"), "a").auth_strategy, "oidc:keycloak");
    }
}
