// SPDX-License-Identifier: BSL-1.1

//! Authentication for the MCP endpoint, including OAuth 2.1 resource tokens.
//!
//! The shared auth middleware only resolves login tokens, admin tokens, and API
//! keys; an OAuth 2.1 resource-bound token (minted by the authorization server
//! for one MCP endpoint) is intentionally rejected there so it cannot be replayed
//! elsewhere. This module performs the resource-token check that belongs at the
//! resource: it reconstructs the canonical resource URL for the request, pins the
//! token's audience to it via
//! [`validate_resource_token`](raisin_rocksdb::AuthService::validate_resource_token),
//! and — on success — resolves the resource owner's real permissions (for RLS)
//! alongside the consented `scope` set (to narrow tool access).

#[cfg(feature = "storage-rocksdb")]
use axum::http::{header::AUTHORIZATION, HeaderMap};

#[cfg(feature = "storage-rocksdb")]
use raisin_models::auth::AuthContext;

#[cfg(feature = "storage-rocksdb")]
use crate::handlers::oauth_as::helpers::issuer_from_request;
#[cfg(feature = "storage-rocksdb")]
use crate::state::AppState;

/// Resolve the effective auth for an MCP request.
///
/// Returns `(auth_context, consented_scopes)`:
/// - If the request carries a valid OAuth 2.1 resource token bound to *this*
///   endpoint's audience, the context is rebuilt from the token's subject with
///   that user's resolved permissions, and `consented_scopes` is the token's
///   `scope` set (space-delimited). This context drives both RLS-scoped data
///   access and (narrowed) tool gating.
/// - Otherwise the middleware-resolved `ext_auth` is returned unchanged (login
///   token, admin token, API key, or anonymous), with no scope narrowing.
///
/// A resource token presented to the wrong endpoint fails the audience check and
/// falls through to `ext_auth` (typically anonymous), so it confers nothing here.
#[cfg(feature = "storage-rocksdb")]
pub(super) async fn resolve_mcp_auth(
    state: &AppState,
    headers: &HeaderMap,
    repo: &str,
    branch: &str,
    slug: &str,
    ext_auth: Option<AuthContext>,
) -> (Option<AuthContext>, Option<Vec<String>>) {
    use raisin_core::PermissionService;

    let Some(token) = bearer_token(headers) else {
        return (ext_auth, None);
    };

    let Some(auth_service) = state.auth_service() else {
        return (ext_auth, None);
    };

    // The audience the authorization server minted and the protected-resource
    // metadata advertises: `{issuer}/mcp/{repo}/{branch}/{slug}`.
    let resource = format!(
        "{}/mcp/{}/{}/{}",
        issuer_from_request(headers),
        repo,
        branch,
        slug
    );

    let claims = match auth_service.validate_resource_token(token, &resource) {
        Ok(claims) => claims,
        // Not a resource token for this endpoint — leave the middleware's result
        // in place (a login token / API key the middleware already resolved, or
        // anonymous).
        Err(_) => return (ext_auth, None),
    };

    // Rebuild the caller's context from their real permissions so RLS scopes data
    // access correctly; the token only narrows which tools are reachable.
    let permission_service = PermissionService::new(state.storage().clone());
    let permissions = permission_service
        .resolve_for_identity_id(&claims.tenant_id, repo, "main", &claims.sub)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(
                user = %claims.sub,
                error = %e,
                "Failed to resolve permissions for OAuth resource-token subject"
            );
            None
        });

    let mut ctx = AuthContext::for_user(&claims.sub).with_email(&claims.email);
    if let Some(perms) = permissions {
        ctx = ctx.with_permissions(perms);
    }

    let consented = claims.scope.as_deref().map(|s| {
        s.split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>()
    });

    (Some(ctx), consented)
}

/// Extract a `Bearer` token from the `Authorization` header.
#[cfg(feature = "storage-rocksdb")]
fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty())
}
