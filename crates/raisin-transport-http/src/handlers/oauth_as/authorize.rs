// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Authorization endpoint (RFC 6749 §4.1.1, OAuth 2.1 profile): `/authorize`.
//!
//! `GET /authorize` validates the request against the registered client and
//! renders a login + consent form. `POST /authorize` authenticates the resource
//! owner through the *existing* identity store (the same credential check and
//! just-in-time user-node provisioning the login handlers use — no second login
//! system), maps the consented scopes onto the identity's
//! `raisin:access_control` roles/groups, issues a single-use authorization code,
//! and redirects back to the client.

use axum::extract::{Form, Query, State};
use axum::http::HeaderMap;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::Extension;
use serde::Deserialize;

use raisin_auth::authserver::{
    AuthServerError, AuthorizationRequest, IdentityGrants, ResourceOwner,
    ValidatedAuthorizationRequest,
};

use super::consent::render_consent_form;
use super::helpers::{
    issuer_from_request, load_tenant_trusted_hosts, mcp_resource_url, oauth_error_response,
    origin_of,
};
use crate::middleware::TenantInfo;
use crate::state::AppState;

/// Query parameters of a `GET /authorize` request (RFC 6749 §4.1.1 + RFC 8707).
#[derive(Debug, Clone, Deserialize)]
pub struct AuthorizeQuery {
    /// Must be `code`.
    pub response_type: String,
    /// The requesting client.
    pub client_id: String,
    /// Where to return the code.
    pub redirect_uri: Option<String>,
    /// Opaque client state, round-tripped on the redirect.
    pub state: Option<String>,
    /// PKCE challenge (required).
    pub code_challenge: Option<String>,
    /// PKCE method (defaults to `S256`).
    pub code_challenge_method: Option<String>,
    /// Requested scope (space-delimited).
    pub scope: Option<String>,
    /// Resource indicator: the MCP server URL the token will target.
    pub resource: Option<String>,
}

impl From<&AuthorizeQuery> for AuthorizationRequest {
    fn from(q: &AuthorizeQuery) -> Self {
        AuthorizationRequest {
            response_type: q.response_type.clone(),
            client_id: q.client_id.clone(),
            redirect_uri: q.redirect_uri.clone(),
            state: q.state.clone(),
            code_challenge: q.code_challenge.clone(),
            code_challenge_method: q.code_challenge_method.clone(),
            scope: q.scope.clone(),
            resource: q.resource.clone(),
        }
    }
}

/// The `POST /authorize` form body: the original request parameters plus the
/// resource owner's credentials.
#[derive(Debug, Clone, Deserialize)]
pub struct AuthorizeForm {
    /// `response_type` (carried through hidden field).
    pub response_type: String,
    /// `client_id`.
    pub client_id: String,
    /// `redirect_uri`.
    pub redirect_uri: Option<String>,
    /// `state`.
    pub state: Option<String>,
    /// `code_challenge`.
    pub code_challenge: Option<String>,
    /// `code_challenge_method`.
    pub code_challenge_method: Option<String>,
    /// `scope`.
    pub scope: Option<String>,
    /// `resource`.
    pub resource: Option<String>,
    /// The repository the MCP resource lives under (carried through hidden
    /// field). Accepted for form compatibility but **not trusted**: the
    /// repository is derived from the canonical `resource` indicator instead, so
    /// a posted value cannot steer provisioning at a different repo than the one
    /// the token is bound to.
    pub repo: String,
    /// The resource owner's email.
    pub email: String,
    /// The resource owner's password.
    pub password: String,
}

impl From<&AuthorizeForm> for AuthorizationRequest {
    fn from(f: &AuthorizeForm) -> Self {
        AuthorizationRequest {
            response_type: f.response_type.clone(),
            client_id: f.client_id.clone(),
            redirect_uri: f.redirect_uri.clone(),
            state: f.state.clone(),
            code_challenge: f.code_challenge.clone(),
            code_challenge_method: f.code_challenge_method.clone(),
            scope: f.scope.clone(),
            resource: f.resource.clone(),
        }
    }
}

/// `GET /authorize` — validate the request and present the login + consent form.
///
/// The repository the consent (and later token) is scoped to is taken from the
/// resource indicator's path (`/mcp/{repo}/{branch}/{slug}`); it is embedded in
/// the form so `POST /authorize` provisions the user node in the right repo.
#[cfg(feature = "storage-rocksdb")]
pub async fn authorize_get(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    headers: HeaderMap,
    Query(query): Query<AuthorizeQuery>,
) -> Response {
    let req = AuthorizationRequest::from(&query);
    let mut validated = match state
        .oauth_server
        .begin_authorization(&tenant_info.tenant_id, &req)
        .await
    {
        Ok((_client, validated)) => validated,
        Err(err) => return oauth_error_response(&err),
    };

    let (repo, _branch, _slug) =
        match canonicalize_resource(&state, &headers, &tenant_info.tenant_id, &mut validated).await
        {
            Ok(parts) => parts,
            Err(err) => return oauth_error_response(&err),
        };

    Html(render_consent_form(&validated, &repo)).into_response()
}

/// Pin the request's resource indicator to this deployment's canonical MCP
/// endpoint URL, returning its `(repo, branch, slug)`.
///
/// RFC 8707 lets the *client* name the resource, and that string used to flow
/// verbatim into the authorization code and then into the token's `aud`
/// (`token.rs`'s `TokenGrant::audience`). The resource server, meanwhile,
/// reconstructs the audience it expects from its own issuer. Two spellings of
/// the same URL therefore produced a token that could never be validated — the
/// request silently degraded to anonymous. Rewriting `resource` here means the
/// audience is always the server's own canonical form, so mint and verify agree
/// by construction.
///
/// It is also a hardening step: a client may only name a resource on this
/// issuer's origin, so it cannot choose an audience for someone else.
#[cfg(feature = "storage-rocksdb")]
async fn canonicalize_resource(
    state: &AppState,
    headers: &HeaderMap,
    tenant_id: &str,
    validated: &mut ValidatedAuthorizationRequest,
) -> Result<(String, String, String), AuthServerError> {
    let (repo, branch, slug) = parse_mcp_resource(&validated.resource).ok_or_else(|| {
        AuthServerError::InvalidTarget(format!(
            "resource '{}' is not an MCP endpoint URL",
            validated.resource
        ))
    })?;

    let tenant_hosts = load_tenant_trusted_hosts(state, tenant_id).await;
    let issuer = issuer_from_request(headers, &tenant_hosts)?;

    let requested_origin = origin_of(&validated.resource).ok_or_else(|| {
        AuthServerError::InvalidTarget(format!(
            "resource '{}' has no usable origin",
            validated.resource
        ))
    })?;
    let issuer_origin = origin_of(&issuer).unwrap_or_else(|| issuer.clone());
    if requested_origin != issuer_origin {
        return Err(AuthServerError::InvalidTarget(format!(
            "resource origin '{requested_origin}' is not this authorization server's issuer \
             '{issuer_origin}'"
        )));
    }

    validated.resource = mcp_resource_url(&issuer, &repo, &branch, &slug);
    Ok((repo, branch, slug))
}

/// `POST /authorize` — authenticate the resource owner, mint a code, redirect.
#[cfg(feature = "storage-rocksdb")]
pub async fn authorize_post(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    headers: HeaderMap,
    Form(form): Form<AuthorizeForm>,
) -> Response {
    let tenant_id = tenant_info.tenant_id.clone();
    let req = AuthorizationRequest::from(&form);

    // Re-validate the request: the client/redirect/PKCE/scope must still hold.
    let mut validated = match state
        .oauth_server
        .begin_authorization(&tenant_id, &req)
        .await
    {
        Ok((_client, validated)) => validated,
        Err(err) => return oauth_error_response(&err),
    };

    // Pin the audience to this issuer's canonical URL before the code is minted.
    // The repo and branch come from the canonical resource, not the form's hidden
    // `repo` field — the resource is what the token is bound to, so provisioning
    // and permission resolution must follow it rather than a value the client
    // could post independently.
    let (repo, branch, _slug) =
        match canonicalize_resource(&state, &headers, &tenant_id, &mut validated).await {
            Ok(parts) => parts,
            Err(err) => return oauth_error_response(&err),
        };

    // Authenticate the resource owner against the existing identity store.
    let owner =
        match authenticate_owner(&state, &tenant_id, &form, &validated, &repo, &branch).await {
            Ok(owner) => owner,
            Err(resp) => return resp,
        };

    // Issue + persist the single-use authorization code.
    let code = match state
        .oauth_server
        .complete_authorization(&tenant_id, &validated, &owner)
        .await
    {
        Ok(code) => code,
        Err(err) => return oauth_error_response(&err),
    };

    // Redirect back to the client with the code (and state, if supplied).
    let mut redirect = url::Url::parse(&validated.redirect_uri)
        .expect("redirect_uri was validated against the client registration");
    redirect.query_pairs_mut().append_pair("code", &code);
    if let Some(state_val) = &validated.state {
        redirect.query_pairs_mut().append_pair("state", state_val);
    }
    Redirect::to(redirect.as_str()).into_response()
}

/// Verify the resource owner's credentials and resolve the scopes they may be
/// granted, reusing the existing identity store, password strategy, user-node
/// provisioning, and permission resolution.
///
/// `repo` and `branch` come from the canonical resource indicator (see
/// [`canonicalize_resource`]), so the user node is provisioned and permissions
/// resolved in the repository the token will actually be bound to.
#[cfg(feature = "storage-rocksdb")]
#[allow(clippy::too_many_arguments)]
async fn authenticate_owner(
    state: &AppState,
    tenant_id: &str,
    form: &AuthorizeForm,
    validated: &ValidatedAuthorizationRequest,
    repo: &str,
    branch: &str,
) -> Result<ResourceOwner, Response> {
    use raisin_auth::authserver::grant_scopes;
    use raisin_auth::strategies::LocalStrategy;
    use raisin_core::PermissionService;

    use crate::handlers::identity_auth::helpers::extract_repos;
    use crate::handlers::identity_auth::user_node::ensure_user_node;

    let repos = extract_repos(state).map_err(|e| e.into_response())?;

    let access_denied =
        || access_denied_response("Invalid email or password, or insufficient consent");

    // Look up the identity and verify the password (same checks as login).
    let identity = match repos.identity.find_by_email(tenant_id, &form.email).await {
        Ok(Some(identity)) => identity,
        Ok(None) => return Err(access_denied()),
        Err(e) => {
            return Err(server_error_response(&format!(
                "failed to query identity: {e}"
            )))
        }
    };

    if !identity.is_active {
        return Err(access_denied());
    }
    let credentials = match identity.local_credentials.as_ref() {
        Some(c) if !c.is_locked() => c,
        _ => return Err(access_denied()),
    };
    if !LocalStrategy::verify_password(&form.password, &credentials.password_hash) {
        let _ = repos
            .identity
            .record_failed_login(tenant_id, &identity.identity_id, 5, 15)
            .await;
        return Err(access_denied());
    }
    let _ = repos
        .identity
        .record_successful_login(tenant_id, &identity.identity_id, "system:oauth-authorize")
        .await;

    // Just-in-time user-node provisioning in the resource's repository.
    let _home = ensure_user_node(
        &repos.storage,
        tenant_id,
        repo,
        &identity.identity_id,
        &identity.email,
        identity.display_name.as_deref(),
        &["viewer".to_string(), "authenticated_user".to_string()],
    )
    .await
    .ok();

    // Resolve the identity's roles/groups, then narrow the requested scopes to
    // what the identity actually holds (consent never widens access).
    let permission_service = &state.permission_service;
    let grants = match permission_service
        .resolve_for_identity_id(tenant_id, repo, "main", &identity.identity_id)
        .await
    {
        Ok(Some(perms)) => IdentityGrants::new(
            perms.effective_roles.clone(),
            perms.groups.clone(),
            perms.is_system_admin,
        ),
        Ok(None) => IdentityGrants::default(),
        Err(e) => {
            return Err(server_error_response(&format!(
                "failed to resolve permissions: {e}"
            )))
        }
    };
    let granted_scopes = grant_scopes(&validated.requested_scopes, &grants);

    Ok(ResourceOwner {
        identity_id: identity.identity_id,
        email: identity.email,
        repository: repo.to_string(),
        branch: branch.to_string(),
        granted_scopes,
    })
}

/// Parse an MCP resource URL (`{base}/mcp/{repo}/{branch}/{slug}`) into its
/// `(repo, branch, slug)` segments.
pub fn parse_mcp_resource(resource: &str) -> Option<(String, String, String)> {
    let url = url::Url::parse(resource).ok()?;
    let segments: Vec<&str> = url.path_segments()?.collect();
    // Find the `mcp` marker and read the following three segments.
    let idx = segments.iter().position(|s| *s == "mcp")?;
    let repo = segments.get(idx + 1)?;
    let branch = segments.get(idx + 2)?;
    let slug = segments.get(idx + 3)?;
    if repo.is_empty() || branch.is_empty() || slug.is_empty() {
        return None;
    }
    Some((repo.to_string(), branch.to_string(), slug.to_string()))
}

/// Render an `access_denied` page (HTTP 403) for a failed owner authentication.
fn access_denied_response(message: &str) -> Response {
    use axum::http::StatusCode;
    (
        StatusCode::FORBIDDEN,
        Html(format!("<h1>Access denied</h1><p>{message}</p>")),
    )
        .into_response()
}

/// Render a `server_error` page (HTTP 500) for an internal failure.
fn server_error_response(detail: &str) -> Response {
    use axum::http::StatusCode;
    tracing::error!(detail, "OAuth authorize internal error");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html(
            "<h1>Server error</h1><p>Unable to process the authorization request.</p>".to_string(),
        ),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mcp_resource_path() {
        let parsed =
            parse_mcp_resource("https://db.example.com/mcp/myrepo/main/assistant").unwrap();
        assert_eq!(
            parsed,
            (
                "myrepo".to_string(),
                "main".to_string(),
                "assistant".to_string()
            )
        );
    }

    #[test]
    fn rejects_non_mcp_resource() {
        assert!(parse_mcp_resource("https://db.example.com/api/foo").is_none());
        assert!(parse_mcp_resource("not a url").is_none());
    }
}
