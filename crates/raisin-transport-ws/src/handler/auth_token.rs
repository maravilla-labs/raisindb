// SPDX-License-Identifier: BSL-1.1

//! JWT authentication for WebSocket connections.
//!
//! Supports two JWT formats:
//! 1. WebSocket JWT (admin users) - validated with the WebSocket secret
//! 2. Identity JWT (identity users) - decoded without validation (already validated when issued)
//!
//! Identity users additionally get their ACL permissions resolved here, the
//! same way the message-based path does in `handlers/auth.rs` (see
//! `handle_authenticate_jwt`). Skipping that step does not fail loudly: the
//! connection authenticates fine, but `AuthContext.resolved_permissions` stays
//! `None`, the RLS filter fail-closes on every row
//! (`raisin-core/src/services/rls_filter/mod.rs:22-82`) and every read comes
//! back as an empty result set. Browsers cannot set WebSocket headers, so the
//! Studio SPA never hit this — but the CLI, SSR and Node SDK all authenticate
//! by header and were reading nothing.

use tracing::{debug, info, warn};

use crate::{connection::ConnectionState, error::WsError};
use raisin_models::{auth::AuthContext, permissions::ResolvedPermissions};

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

            // Resolve ACL permissions in the SAME tenant/repo this connection
            // was just constructed with (see the tenant/repo note on
            // `resolve_identity_permissions`).
            let resolved = resolve_identity_permissions(
                state,
                &conn_state.tenant_id,
                conn_state.repository.as_deref(),
                &claims.sub,
            )
            .await;

            let mut auth_context = AuthContext::for_user(&claims.sub);
            if !claims.email.is_empty() {
                auth_context = auth_context.with_email(claims.email.clone());
            }
            if let Some(home) = claims.home.clone() {
                auth_context = auth_context.with_home(home);
            }
            auth_context = auth_context.with_permissions(resolved);
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

                // NO permission resolution here, deliberately - unlike the
                // verified branch above.
                //
                // This branch decoded the JWT payload WITHOUT checking its
                // signature, so `sub` is an unauthenticated claim: anyone who
                // can reach a dev-mode server can assert any identity. Calling
                // resolve_identity_permissions here would hand them that
                // identity's real roles. Leaving `resolved_permissions` as None
                // keeps this path fail-closed (the RLS filter denies every row,
                // rls_filter/mod.rs:22-82) - which is what it has always been,
                // by accident rather than by intent until now.
                //
                // Dev clients that need real permissions should present a
                // properly signed token and take branch 2.
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

/// Resolve a header-authenticated identity user's ACL permissions.
///
/// Mirrors `handlers/auth.rs::handle_authenticate_jwt` (lines 200-213): resolve
/// against the `raisin:access_control` workspace by identity id, and on miss
/// warn and fall back to an *empty* permission set rather than to `None`.
/// Empty and `None` both deny, but only empty says "we asked and the answer was
/// nothing" — `None` is the state that silently turns every query into `200 []`.
///
/// TENANT/REPO: `handle_authenticate_jwt` deliberately takes tenant/repo from
/// the connection URL, because by the time that message arrives the connection
/// already exists and the URL is what scoped it. The header path cannot do the
/// same: it *builds* the `ConnectionState`, and `handle_socket`
/// (`handler/socket.rs:59`) does not hand the URL's tenant/repo to
/// `authenticate_with_token` — the token claims are the only source available.
/// So we resolve against the connection state we just built, which keeps the
/// invariant that actually matters: permissions are resolved in exactly the
/// tenant/repo the connection will run its queries in. Claims and URL
/// disagreeing is a pre-existing property of this path (the claims already win
/// for routing); this change does not widen it, and fixing it belongs in
/// `socket.rs`, which owns both values.
async fn resolve_identity_permissions<S, B>(
    state: &WsState<S, B>,
    tenant_id: &str,
    repository: Option<&str>,
    user_id: &str,
) -> ResolvedPermissions
where
    S: raisin_storage::Storage,
    B: raisin_binary::BinaryStorage,
{
    let repo_id = repository.unwrap_or("default");

    // "main", not the repository's default branch: the raisin:access_control
    // workspace pins its own branch to main
    // (raisin-core/global_workspaces/access_control.yaml), so roles and users
    // live there regardless of what the repository defaults to. Every other
    // resolve_for_identity_id call site passes the same literal; see the
    // ACCESS_CONTROL_BRANCH note in
    // raisin-transport-http/src/handlers/identity_auth/user_node.rs.
    let permission_service = raisin_core::PermissionService::new(state.storage.clone());
    match permission_service
        .resolve_for_identity_id(tenant_id, repo_id, "main", user_id)
        .await
    {
        Ok(Some(resolved)) => {
            info!(
                user_id = %user_id,
                tenant_id = %tenant_id,
                repo_id = %repo_id,
                roles = ?resolved.effective_roles,
                "Resolved permissions for header-authenticated identity user"
            );
            resolved
        }
        Ok(None) => {
            // No raisin:User node matches this identity id. The orphaned-ACL-node
            // incident looked exactly like this: the node existed at its
            // email-derived path but still carried the *old* identity's user_id.
            warn!(
                user_id = %user_id,
                tenant_id = %tenant_id,
                repo_id = %repo_id,
                "No permissions found for header-authenticated user; connection will read nothing"
            );
            ResolvedPermissions::empty(user_id)
        }
        Err(e) => {
            warn!(
                user_id = %user_id,
                tenant_id = %tenant_id,
                repo_id = %repo_id,
                error = %e,
                "Failed to resolve permissions for header-authenticated user; denying"
            );
            ResolvedPermissions::empty(user_id)
        }
    }
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
