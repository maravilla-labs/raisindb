// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! OpenID Connect login: the authorization redirect and the callback.
//!
//! # The shape of the flow
//!
//! ```text
//!   browser                RaisinDB                     provider
//!      |  GET /auth/oidc/{p}   |                            |
//!      |---------------------->|  mint verifier + nonce,    |
//!      |                       |  seal them into `state`    |
//!      |<--- 302 authorize ----|                            |
//!      |------------------------------------------------->  |
//!      |                       |            (user signs in)  |
//!      |<-- 302 callback?code=&state= ---------------------  |
//!      |--- GET callback ----->|  open state, redeem code,   |
//!      |                       |  verify id_token  --------> |
//!      |<-- 302 app#tokens ----|                             |
//! ```
//!
//! Nothing is stored between the two requests. The PKCE verifier and the nonce
//! travel inside the sealed `state` parameter, so the callback can be served by
//! a different node than the one that started the login. See
//! [`raisin_auth::strategies::OidcLoginState`].
//!
//! # Two different redirect URIs
//!
//! They are easy to confuse and mean opposite things. The provider's callback
//! is `AuthProviderConfig::redirect_uri`, fixed configuration, registered with
//! the provider, and sent byte-identically at both the authorization and the
//! token endpoint. The application's landing page arrives as the `redirect_uri`
//! query parameter, varies per login, and is allow-listed in `redirect.rs`
//! because whoever receives it receives the tokens.

#[cfg(feature = "storage-rocksdb")]
mod identity;
mod linking;
#[cfg(feature = "storage-rocksdb")]
mod provider;
mod redirect;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    Extension, Json,
};
use serde::Deserialize;

#[cfg(feature = "storage-rocksdb")]
use raisin_auth::strategies::OidcLoginState;

use crate::error::ApiError;
use crate::state::AppState;

#[cfg(feature = "storage-rocksdb")]
use super::helpers::{build_auth_response, create_session, extract_repos, generate_tokens};
#[cfg(feature = "storage-rocksdb")]
use super::user_node::ensure_user_node;
#[cfg(feature = "storage-rocksdb")]
use provider::{build_strategy, load_tenant_config, master_key};

/// Roles granted to a person provisioned into a repository by an OIDC login.
///
/// The same pair local registration uses. Mapping a provider's group claims
/// onto RaisinDB roles is a separate feature; until it exists, an OIDC login
/// must not confer more than a local one.
#[cfg(feature = "storage-rocksdb")]
const DEFAULT_ROLES: [&str; 2] = ["viewer", "authenticated_user"];

/// Query parameters for `GET /auth/oidc/{provider}`.
#[derive(Debug, Deserialize)]
pub struct OidcAuthorizeQuery {
    /// Where to send the browser after a successful login. Omit to receive the
    /// tokens as JSON instead. Allow-listed; see [`redirect`].
    #[serde(alias = "redirect_url")]
    pub redirect_uri: Option<String>,
    /// Repository to provision the user into, so the issued token is
    /// repo-scoped and carries a home path.
    pub repo: Option<String>,
}

/// Query parameters for `GET /auth/oidc/{provider}/callback`.
///
/// Every field is optional because this endpoint receives provider *errors*
/// too, and those carry no `code`. Declaring `code` as required would answer a
/// legitimate "user pressed cancel" with a deserialization failure instead of
/// the provider's own message.
#[derive(Debug, Deserialize)]
pub struct OidcCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

/// Start an OIDC login.
///
/// # Endpoint
/// `GET /auth/oidc/{provider}`
#[cfg(feature = "storage-rocksdb")]
pub async fn oidc_authorize(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<crate::middleware::TenantInfo>,
    Path(provider): Path<String>,
    Query(query): Query<OidcAuthorizeQuery>,
) -> Result<Redirect, ApiError> {
    let tenant_id = &tenant_info.tenant_id;
    let config = load_tenant_config(&state, tenant_id).await?;

    let app_redirect = redirect::resolve_app_redirect(&config, query.redirect_uri.as_deref())?;
    let strategy = build_strategy(&state, tenant_id, &config, &provider).await?;
    let master_key = master_key(&state)?;

    let login = strategy
        .begin_login(tenant_id, &master_key, app_redirect, query.repo.clone())
        .map_err(|e| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "OIDC_CONFIGURATION_ERROR",
                format!("Could not start the login: {e}"),
            )
        })?;

    tracing::info!(
        tenant_id = %tenant_id,
        provider = %provider,
        repo = ?query.repo,
        "starting an OIDC login"
    );

    Ok(Redirect::to(&login.authorization_url))
}

/// Finish an OIDC login.
///
/// # Endpoint
/// `GET /auth/oidc/{provider}/callback`
///
/// Answers with a 302 to the application's landing page, carrying the tokens in
/// the URL **fragment**, or with JSON when the login named no landing page. The
/// fragment is not sent to any server, so the tokens stay out of access logs,
/// out of `Referer` on the next navigation, and out of every proxy in between.
/// This is the same handover the magic-link verifier performs.
#[cfg(feature = "storage-rocksdb")]
pub async fn oidc_callback(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<crate::middleware::TenantInfo>,
    Path(provider): Path<String>,
    Query(query): Query<OidcCallbackQuery>,
) -> Result<Response, ApiError> {
    // The provider refused, or the person pressed cancel. Its message is more
    // informative than anything we could infer, so it is passed through.
    if let Some(error) = &query.error {
        let message = query
            .error_description
            .as_deref()
            .unwrap_or("Authorization failed");
        tracing::info!(provider = %provider, error = %error, "OIDC provider refused the login");
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, error, message));
    }

    let tenant_id = &tenant_info.tenant_id;
    let code = query
        .code
        .as_deref()
        .ok_or_else(|| bad_callback("the callback carried no authorization code"))?;
    let raw_state = query
        .state
        .as_deref()
        .ok_or_else(|| bad_callback("the callback carried no state parameter"))?;

    let master_key = master_key(&state)?;
    // Opening the state proves this callback belongs to a login *we* started,
    // for this tenant and this provider, within the last few minutes. It is the
    // CSRF check, and it happens before the code is spent.
    let login_state = OidcLoginState::open(raw_state, &master_key, tenant_id, &provider)
        .map_err(|_| bad_callback("the state parameter is not valid or has expired"))?;

    let config = load_tenant_config(&state, tenant_id).await?;
    let strategy = build_strategy(&state, tenant_id, &config, &provider).await?;

    let assertion = strategy
        .complete_login(&login_state, code)
        .await
        .map_err(|e| {
            tracing::warn!(provider = %provider, error = %e, "OIDC login failed");
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "OIDC_LOGIN_FAILED",
                format!("{e}"),
            )
        })?;

    let repos = extract_repos(&state)?;
    let strategy_id = format!("oidc:{provider}");
    let resolved = identity::resolve_identity(&repos, tenant_id, &strategy_id, &assertion).await?;
    let identity = resolved.identity;

    // The second element is the session's own expiry. The response reports the
    // ACCESS TOKEN's expiry instead, which `build_auth_response` derives from
    // the minted tokens, so the two cannot drift apart.
    let (session, _session_expires_at) = create_session(
        &repos.session,
        tenant_id,
        &identity.identity_id,
        &strategy_id,
        false,
        "system:oidc_login",
    )
    .await?;

    // Just-in-time provisioning, exactly as the local login does it. A failure
    // here is logged and not fatal: the person is authenticated either way, and
    // refusing the login would be a worse answer than a token with no home.
    let home = match &login_state.repo {
        Some(repo) => ensure_user_node(
            &repos.storage,
            tenant_id,
            repo,
            &identity.identity_id,
            &identity.email,
            identity.display_name.as_deref(),
            &DEFAULT_ROLES.map(String::from),
        )
        .await
        .map_err(|e| {
            tracing::warn!(
                identity_id = %identity.identity_id,
                repo = %repo,
                error = %e,
                "could not ensure the user node during an OIDC login"
            );
        })
        .ok(),
        None => None,
    };

    // Async, and policy-aware: it loads the tenant's session settings and mints
    // with those lifetimes. An OIDC session must expire on the same schedule a
    // local login's does, so this is the helper to call rather than any
    // lifetime-unaware variant.
    let tokens = generate_tokens(
        &state,
        &identity,
        &session,
        login_state.repo.as_deref(),
        home.as_deref(),
    )
    .await?;
    let response = build_auth_response(&identity, tokens, home);

    tracing::info!(
        identity_id = %identity.identity_id,
        provider = %provider,
        created = resolved.created,
        repo = ?login_state.repo,
        "OIDC login succeeded"
    );

    match &login_state.redirect_uri {
        Some(base) => {
            let target = format!(
                "{}#access_token={}&refresh_token={}&expires_at={}",
                base,
                urlencoding::encode(&response.access_token),
                urlencoding::encode(&response.refresh_token),
                response.expires_at
            );
            Ok(Redirect::to(&target).into_response())
        }
        None => Ok(Json(response).into_response()),
    }
}

fn bad_callback(message: &str) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "INVALID_OIDC_CALLBACK", message)
}
