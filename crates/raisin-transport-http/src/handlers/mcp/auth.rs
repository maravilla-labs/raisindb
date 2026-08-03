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
use crate::handlers::oauth_as::helpers::{
    issuer_from_request, load_tenant_trusted_hosts, mcp_resource_url,
};
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
    tenant_id: &str,
    repo: &str,
    branch: &str,
    slug: &str,
    ext_auth: Option<AuthContext>,
) -> (Option<AuthContext>, Option<Vec<String>>) {
    use raisin_core::PermissionService;

    let Some(token) = bearer_token(headers) else {
        // No credential at all — normal for a `public` MCP server.
        tracing::debug!(%repo, %branch, %slug, "MCP request carries no bearer token");
        return (ext_auth, None);
    };

    let Some(auth_service) = state.auth_service() else {
        tracing::warn!(
            %repo, %branch, %slug,
            "MCP request presented a bearer token but no auth service is configured; \
             cannot validate an OAuth resource token"
        );
        return (ext_auth, None);
    };

    // The audience the authorization server minted and the protected-resource
    // metadata advertises: `{issuer}/mcp/{repo}/{branch}/{slug}`. The issuer host
    // is validated against the tenant's trusted-host allowlist; an untrusted host
    // cannot mint a matching audience, so the token confers nothing (fail closed).
    let tenant_hosts = load_tenant_trusted_hosts(state, tenant_id).await;
    let issuer = match issuer_from_request(headers, &tenant_hosts) {
        Ok(issuer) => issuer,
        Err(e) => {
            tracing::warn!(
                %repo, %branch, %slug,
                host = ?headers.get("host").and_then(|h| h.to_str().ok()),
                forwarded_host = ?headers.get("x-forwarded-host").and_then(|h| h.to_str().ok()),
                error = %e,
                "Cannot derive a trusted issuer for an MCP request; OAuth resource \
                 token not validated"
            );
            return (ext_auth, None);
        }
    };
    let resource = mcp_resource_url(&issuer, repo, branch, slug);

    let claims = match auth_service.validate_resource_token(token, &resource) {
        Ok(claims) => claims,
        // Not a resource token for this endpoint — leave the middleware's result
        // in place (a login token / API key the middleware already resolved, or
        // anonymous). Log both audiences: an `aud` that differs only in spelling
        // is exactly the failure this whole path once degraded to anonymous on,
        // with nothing in the log to say so.
        Err(e) => {
            tracing::warn!(
                expected_audience = %resource,
                presented_audience = ?peek_unverified_audience(token),
                error = %e,
                "OAuth resource token rejected for MCP endpoint; falling back to the \
                 middleware-resolved context"
            );
            return (ext_auth, None);
        }
    };

    // Rebuild the caller's context from their real permissions so RLS scopes data
    // access correctly; the token only narrows which tools are reachable.
    let permission_service = &state.permission_service;
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

    let consented = claims
        .scope
        .as_deref()
        .map(|s| s.split_whitespace().map(str::to_string).collect::<Vec<_>>());

    (Some(ctx), consented)
}

/// The RFC 9728 protected-resource metadata URL to advertise in a `401`
/// challenge for this endpoint, or `None` when no trusted issuer can be derived.
///
/// This is what lets an MCP client discover *where* to authenticate: it fetches
/// the document, finds this deployment's authorization server, and starts the
/// OAuth flow. Without it a client only sees a generic failure and has no way to
/// know a token would help.
#[cfg(feature = "storage-rocksdb")]
pub(super) async fn resource_metadata_challenge(
    state: &AppState,
    headers: &HeaderMap,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    slug: &str,
) -> Option<String> {
    use crate::handlers::oauth_as::helpers::mcp_resource_metadata_url;

    let tenant_hosts = load_tenant_trusted_hosts(state, tenant_id).await;
    let issuer = issuer_from_request(headers, &tenant_hosts).ok()?;
    Some(mcp_resource_metadata_url(&issuer, repo, branch, slug))
}

/// Read a JWT's `aud` claim **without verifying the signature**, for logging.
///
/// Diagnostics only: when the audience check fails, knowing which audience the
/// token actually carries — next to the one this endpoint expected — is the
/// difference between a one-line fix and a day of guessing. The value is never
/// used in any authorization decision; the signature is unchecked, so it is
/// attacker-controlled.
#[cfg(feature = "storage-rocksdb")]
fn peek_unverified_audience(token: &str) -> Option<String> {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;

    let payload = token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    claims.get("aud")?.as_str().map(str::to_string)
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

#[cfg(all(test, feature = "storage-rocksdb"))]
mod tests {
    use super::*;
    use crate::handlers::oauth_as::helpers::mcp_resource_url;
    use raisin_auth::authserver::TokenGrant;
    use raisin_rocksdb::{AdminUserStore, AuthService, RocksDBConfig, RocksDBStorage};

    fn service() -> (tempfile::TempDir, AuthService) {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config = RocksDBConfig::default().with_path(temp_dir.path());
        let storage = RocksDBStorage::with_config(config).unwrap();
        let store = AdminUserStore::new(storage.db().clone());
        (temp_dir, AuthService::new(store, "test-secret".to_string()))
    }

    fn grant(audience: &str) -> TokenGrant {
        TokenGrant {
            identity_id: "id-1".to_string(),
            email: "u@example.com".to_string(),
            tenant_id: "tenant-a".to_string(),
            repository: "studio".to_string(),
            branch: "main".to_string(),
            audience: audience.to_string(),
            scope: "reader".to_string(),
            client_id: "client-1".to_string(),
        }
    }

    /// The regression this whole change exists for: `/authorize` derives the
    /// issuer on one request and the MCP endpoint derives it again on another.
    /// If the two derivations differ only in *spelling*, the audience check must
    /// still pass — before canonicalization it did not, and the request silently
    /// degraded to anonymous with nothing in the log.
    #[test]
    fn audience_survives_a_differently_spelled_issuer() {
        let (_dir, svc) = service();

        // Minted at /authorize from one spelling of the origin ...
        let minted = mcp_resource_url("https://RDB.Example.Test:443/", "studio", "main", "studio");
        let (token, _) = svc.mint_oauth_access_token(&grant(&minted)).unwrap();

        // ... verified at the MCP endpoint from another.
        let expected = mcp_resource_url("https://rdb.example.test", "studio", "main", "studio");
        let claims = svc
            .validate_resource_token(&token, &expected)
            .expect("canonicalization makes both spellings agree");
        assert_eq!(claims.sub, "id-1");
        assert_eq!(claims.scope.as_deref(), Some("reader"));
    }

    /// Canonicalization must not weaken the RFC 8707 binding: a token minted for
    /// one MCP endpoint still confers nothing at another.
    #[test]
    fn audience_still_pins_to_one_endpoint() {
        let (_dir, svc) = service();
        let minted = mcp_resource_url("https://db.example.com", "studio", "main", "studio");
        let (token, _) = svc.mint_oauth_access_token(&grant(&minted)).unwrap();

        for (repo, branch, slug) in [
            ("other", "main", "studio"),
            ("studio", "dev", "studio"),
            ("studio", "main", "other"),
        ] {
            let other = mcp_resource_url("https://db.example.com", repo, branch, slug);
            assert!(
                svc.validate_resource_token(&token, &other).is_err(),
                "token must not validate for /mcp/{repo}/{branch}/{slug}"
            );
        }
    }

    #[test]
    fn peeked_audience_reports_the_presented_value() {
        let (_dir, svc) = service();
        let minted = mcp_resource_url("https://db.example.com", "studio", "main", "studio");
        let (token, _) = svc.mint_oauth_access_token(&grant(&minted)).unwrap();

        assert_eq!(
            peek_unverified_audience(&token).as_deref(),
            Some(minted.as_str())
        );
        assert_eq!(peek_unverified_audience("not-a-jwt"), None);
    }
}
