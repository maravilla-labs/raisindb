// SPDX-License-Identifier: BSL-1.1

//! JWT authentication for WebSocket connections.
//!
//! Supports two JWT formats:
//! 1. WebSocket JWT (admin users) - validated with the WebSocket secret
//! 2. Identity JWT (identity users) - decoded without validation (already validated when issued)

use tracing::{debug, warn};

use crate::{connection::ConnectionState, error::WsError};
use raisin_models::auth::AuthContext;

use super::state::WsState;

/// Authenticate using a JWT token
pub(super) async fn authenticate_with_token<S, B>(
    state: &WsState<S, B>,
    token: &str,
) -> Result<ConnectionState, WsError>
where
    S: raisin_storage::Storage,
    B: raisin_binary::BinaryStorage,
{
    // 1. Try WebSocket JWT validation first (admin users)
    if let Ok(claims) = state.auth_service.validate_access_token(token) {
        debug!("Authenticated as admin user: {}", claims.sub);

        let mut conn_state = ConnectionState::new(
            claims.tenant_id.clone(),
            claims.repository.clone(),
            state.config.max_concurrent_ops,
            state.config.initial_credits,
        );

        conn_state.set_user_id(claims.sub.clone());

        // Admin users get system auth context (bypasses RLS)
        conn_state.set_auth_context(AuthContext::system());

        return Ok(conn_state);
    }

    // 2. Try cryptographic user-token validation via RocksDB auth service (secure)
    #[cfg(feature = "storage-rocksdb")]
    if let Some(ref auth_service) = state.rocksdb_auth_service {
        if let Ok(claims) = auth_service.validate_user_token(token) {
            debug!("Authenticated as identity user (verified): {}", claims.sub);

            let tenant = if claims.tenant_id.is_empty() {
                "default".to_string()
            } else {
                claims.tenant_id.clone()
            };
            let repo = claims
                .repository
                .clone()
                .or_else(|| Some("default".to_string()));

            let mut conn_state = ConnectionState::new(
                tenant,
                repo,
                state.config.max_concurrent_ops,
                state.config.initial_credits,
            );

            conn_state.set_user_id(claims.sub.clone());

            let mut auth_context = AuthContext::for_user(&claims.sub);
            if !claims.email.is_empty() {
                auth_context = auth_context.with_email(claims.email.clone());
            }
            if let Some(home) = claims.home.clone() {
                auth_context = auth_context.with_home(home);
            }
            conn_state.set_auth_context(auth_context);

            return Ok(conn_state);
        }
    }

    // 3. Insecure fallback: decode JWT payload without signature verification.
    //    Only allowed in dev-mode.
    if state.config.dev_mode {
        warn!("Using insecure JWT decode fallback (dev-mode only)");
        match decode_identity_jwt(token) {
            Ok((sub, email, tenant_id, repository, home)) => {
                debug!(
                    "Authenticated as identity user (unverified, dev-mode): {}",
                    sub
                );

                let tenant = tenant_id.unwrap_or_else(|| "default".to_string());
                let repo = repository.or_else(|| Some("default".to_string()));

                let mut conn_state = ConnectionState::new(
                    tenant,
                    repo,
                    state.config.max_concurrent_ops,
                    state.config.initial_credits,
                );

                conn_state.set_user_id(sub.clone());

                let mut auth_context = AuthContext::for_user(&sub);
                if let Some(email) = email {
                    auth_context = auth_context.with_email(email);
                }
                if let Some(home) = home {
                    auth_context = auth_context.with_home(home);
                }
                conn_state.set_auth_context(auth_context);

                return Ok(conn_state);
            }
            Err(e) => {
                warn!("Failed to decode identity JWT (dev-mode fallback): {}", e);
            }
        }
    }

    Err(WsError::AuthError(crate::auth::AuthError::InvalidToken(
        "Invalid JWT token".to_string(),
    )))
}

/// Decoded JWT identity claims: (sub, email, tenant_id, repository, home)
type JwtIdentityClaims = (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Decode identity JWT claims without cryptographic validation
///
/// Returns (sub, email, tenant_id, repository, home) on success
fn decode_identity_jwt(token: &str) -> Result<JwtIdentityClaims, String> {
    // Split JWT into parts
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("Invalid JWT format - expected 3 parts".to_string());
    }

    // Decode the payload (second part)
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|e| format!("Failed to decode JWT payload: {}", e))?;

    let payload: serde_json::Value = serde_json::from_slice(&payload_bytes)
        .map_err(|e| format!("Failed to parse JWT payload: {}", e))?;

    let sub = payload
        .get("sub")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'sub' claim in JWT")?
        .to_string();

    let email = payload
        .get("email")
        .and_then(|v| v.as_str())
        .map(String::from);

    let tenant_id = payload
        .get("tenant_id")
        .and_then(|v| v.as_str())
        .map(String::from);

    let repository = payload
        .get("repository")
        .and_then(|v| v.as_str())
        .map(String::from);

    let home = payload
        .get("home")
        .and_then(|v| v.as_str())
        .map(String::from);

    Ok((sub, email, tenant_id, repository, home))
}
