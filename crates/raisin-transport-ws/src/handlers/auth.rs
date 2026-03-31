// SPDX-License-Identifier: BSL-1.1

//! Authentication handlers

use parking_lot::RwLock;
use std::sync::Arc;
use tracing::{info, warn};

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{
        AuthenticateJwtPayload, AuthenticateJwtResponse, AuthenticatePayload, AuthenticateResponse,
        RefreshTokenPayload, RequestEnvelope, ResponseEnvelope,
    },
};

/// Handle authentication request
pub async fn handle_authenticate<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage,
    B: raisin_binary::BinaryStorage,
{
    let payload: AuthenticatePayload = serde_json::from_value(request.payload.clone())?;

    // Get tenant/repository from connection state (set from URL path)
    let (conn_tenant_id, conn_repository) = {
        let conn = connection_state.read();
        (conn.tenant_id.clone(), conn.repository.clone())
    };

    // Authenticate with the storage backend (if RocksDB with auth_service)
    #[cfg(feature = "storage-rocksdb")]
    {
        if let Some(ref auth_service) = state.rocksdb_auth_service {
            // Verify credentials (using Console interface for WebSocket auth)
            let (admin_user, _db_token) = auth_service
                .authenticate(
                    &conn_tenant_id,
                    &payload.username,
                    &payload.password,
                    raisin_models::admin_user::AdminInterface::Console,
                )
                .map_err(|e| {
                    WsError::AuthError(crate::auth::AuthError::InvalidToken(e.to_string()))
                })?;

            // Generate JWT token pair
            let token_pair = state.auth_service.generate_token_pair(
                admin_user.user_id.clone(),
                conn_tenant_id.clone(),
                conn_repository.clone(),
            )?;

            // Update connection state with user ID (tenant/repo already set from URL)
            {
                let mut conn = connection_state.write();
                conn.set_user_id(admin_user.user_id.clone());
            }

            info!(
                user_id = %admin_user.user_id,
                tenant_id = %conn_tenant_id,
                repository = ?conn_repository,
                "User authenticated via WebSocket"
            );

            let response = AuthenticateResponse {
                access_token: token_pair.access_token,
                refresh_token: token_pair.refresh_token,
                expires_in: token_pair.expires_in,
            };

            return Ok(Some(ResponseEnvelope::success(
                request.request_id,
                serde_json::to_value(response)?,
            )));
        }
    }

    // Fallback: no auth service available
    Err(WsError::AuthError(crate::auth::AuthError::InvalidToken(
        "Authentication not configured".to_string(),
    )))
}

/// Handle token refresh request
pub async fn handle_refresh_token<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage,
    B: raisin_binary::BinaryStorage,
{
    let payload: RefreshTokenPayload = serde_json::from_value(request.payload.clone())?;

    // Refresh the access token
    let token_pair = state
        .auth_service
        .refresh_access_token(&payload.refresh_token)?;

    let response = AuthenticateResponse {
        access_token: token_pair.access_token,
        refresh_token: token_pair.refresh_token,
        expires_in: token_pair.expires_in,
    };

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(response)?,
    )))
}

/// Handle JWT authentication request (identity users)
///
/// Used by SPAs and clients that have already obtained a JWT token
/// via HTTP API (e.g., /auth/{repo}/login). The JWT is decoded and
/// permissions are resolved from the raisin:access_control workspace.
pub async fn handle_authenticate_jwt<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage,
    B: raisin_binary::BinaryStorage,
{
    let payload: AuthenticateJwtPayload = serde_json::from_value(request.payload.clone())?;

    // Validate JWT cryptographically and extract user identity
    // SECURITY: Must use proper signature verification to prevent token forgery
    let (user_id, email, home) = {
        #[cfg(feature = "storage-rocksdb")]
        {
            if let Some(ref auth_service) = state.rocksdb_auth_service {
                // SECURE: Cryptographic JWT validation with signature verification
                let claims = auth_service
                    .validate_user_token(&payload.token)
                    .map_err(|e| {
                        WsError::AuthError(crate::auth::AuthError::InvalidToken(e.to_string()))
                    })?;
                (claims.sub, Some(claims.email), claims.home)
            } else if state.config.dev_mode {
                // Insecure fallback: decode-only (dev-mode only)
                warn!("Using insecure JWT decode — no auth service available (dev-mode)");
                let (uid, email, _, _, home) = decode_identity_jwt(&payload.token)
                    .map_err(|e| WsError::AuthError(crate::auth::AuthError::InvalidToken(e)))?;
                (uid, email, home)
            } else {
                return Err(WsError::AuthError(crate::auth::AuthError::InvalidToken(
                    "Authentication service not available".to_string(),
                )));
            }
        }
        #[cfg(not(feature = "storage-rocksdb"))]
        {
            if state.config.dev_mode {
                // Insecure fallback: decode-only without rocksdb feature (dev-mode only)
                warn!("Using insecure JWT decode — no storage-rocksdb feature (dev-mode)");
                let (uid, email, _, _, home) = decode_identity_jwt(&payload.token)
                    .map_err(|e| WsError::AuthError(crate::auth::AuthError::InvalidToken(e)))?;
                (uid, email, home)
            } else {
                return Err(WsError::AuthError(crate::auth::AuthError::InvalidToken(
                    "JWT authentication requires storage-rocksdb feature or --dev-mode".to_string(),
                )));
            }
        }
    };

    // Get tenant/repo from connection state (set from URL path)
    // This is the authoritative source - the connection URL determines the context
    let (conn_tenant_id, conn_repository) = {
        let conn = connection_state.read();
        (conn.tenant_id.clone(), conn.repository.clone())
    };

    let repo_id = conn_repository
        .clone()
        .unwrap_or_else(|| "default".to_string());

    info!(
        user_id = %user_id,
        tenant_id = %conn_tenant_id,
        repo_id = %repo_id,
        "JWT authentication request"
    );

    // Resolve permissions from raisin:access_control workspace
    let permission_service = raisin_core::PermissionService::new(state.storage.clone());
    let resolved = permission_service
        .resolve_for_identity_id(&conn_tenant_id, &repo_id, "main", &user_id)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| {
            warn!(
                "No permissions found for user {} in {}/{}",
                user_id, conn_tenant_id, repo_id
            );
            raisin_models::permissions::ResolvedPermissions::empty(&user_id)
        });

    let effective_roles = resolved.effective_roles.clone();

    // Build AuthContext with permissions and home path for REL conditions
    let mut auth_context = raisin_models::auth::AuthContext::for_user(&user_id);
    if let Some(ref e) = email {
        auth_context = auth_context.with_email(e.clone());
    }
    if let Some(ref h) = home {
        auth_context = auth_context.with_home(h.clone());
    }
    auth_context = auth_context.with_permissions(resolved);

    // Update connection state with user ID and auth context
    // (tenant/repo already set from URL path - don't overwrite)
    {
        let mut conn = connection_state.write();
        conn.set_user_id(user_id.clone());
        conn.set_auth_context(auth_context.clone());
        info!(
            user_id = %user_id,
            auth_context_user_id = ?auth_context.user_id,
            "Setting auth_context on connection state"
        );
    }

    info!(
        user_id = %user_id,
        roles = ?effective_roles,
        "User authenticated via JWT"
    );

    let response = AuthenticateJwtResponse {
        user_id,
        roles: effective_roles,
    };

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(response)?,
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
