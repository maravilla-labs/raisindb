// SPDX-License-Identifier: BSL-1.1

//! `GET /api/integrations/{repo}/oauth/callback` — finish an outbound OAuth flow.
//!
//! This endpoint is reached by a browser redirect from the provider, so it
//! carries no bearer token. It is authenticated by the single-use, TTL'd
//! `state` value minted by the authenticated `oauth/start` call.

use axum::{
    extract::{Path, Query, State},
    response::Redirect,
};
use base64::Engine as _;
use raisin_crypto::SecretBox;
use serde::Deserialize;
use serde_json::{json, Value};

use super::state_store::{now_secs, OAUTH_STATE_STORE};
use super::{
    connected_accounts, decrypt_client_secret, json_str, oauth_config, set_connected_accounts,
};
use crate::error::ApiError;
use crate::state::AppState;

/// Query parameters returned by the provider on redirect.
#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    /// Provider-supplied human-readable detail (Microsoft puts the `AADSTS…`
    /// code here). Surfaced to the console; never trusted as markup.
    pub error_description: Option<String>,
}

/// Clamp a provider string to `max` chars so a hostile or verbose provider
/// cannot blow up the redirect URL. Char-based, so it never splits UTF-8.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
}

/// Exchange the authorization code for tokens, persist an encrypted account
/// entry, and redirect back to the admin console.
pub async fn callback(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    Query(q): Query<CallbackQuery>,
) -> Result<Redirect, ApiError> {
    // A provider-side refusal is a normal outcome of a user-facing flow, not a
    // server fault. Returning an ApiError rendered a raw JSON blob inside the
    // popup; redirect back to the console instead so the operator sees the
    // provider's own code (e.g. AADSTS50194) in context.
    if let Some(err) = q.error {
        return Ok(Redirect::to(&format!(
            "/{repo}/integrations?oauth_error={}&oauth_error_description={}",
            urlencoding::encode(&err),
            urlencoding::encode(&truncate(q.error_description.as_deref().unwrap_or(""), 300)),
        )));
    }
    let code = q
        .code
        .ok_or_else(|| ApiError::validation_failed("missing authorization code"))?;
    let state_token = q
        .state
        .ok_or_else(|| ApiError::validation_failed("missing state"))?;

    let entry = OAUTH_STATE_STORE
        .consume(&state_token, now_secs())
        .ok_or_else(|| {
            ApiError::new(
                axum::http::StatusCode::BAD_REQUEST,
                "INVALID_STATE",
                "OAuth state is unknown or expired",
            )
        })?;
    // Defend against a state minted for a different repo being replayed here.
    if entry.repo != repo {
        return Err(ApiError::new(
            axum::http::StatusCode::BAD_REQUEST,
            "INVALID_STATE",
            "OAuth state does not match this repository",
        ));
    }

    let master_key = state.get_master_key()?;
    let svc = super::config_service(&state, &entry.tenant_id, &entry.repo, "integration-oauth");
    let mut node = svc
        .get_by_path(&entry.integration_path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(entry.integration_path.clone()))?;

    let cfg = oauth_config(&node);
    let token_url = json_str(&cfg, "token_url").ok_or_else(|| {
        ApiError::validation_failed("integration oauth_config.token_url is missing")
    })?;
    let client_id = json_str(&cfg, "client_id").ok_or_else(|| {
        ApiError::validation_failed("integration oauth_config.client_id is missing")
    })?;
    // Must resolve identically to `oauth/start` — the provider rejects a token
    // exchange whose redirect_uri differs from the authorize request's.
    let redirect_uri = super::resolve_redirect_uri(&cfg, &repo);
    let client_secret = decrypt_client_secret(&node, &master_key);
    let provider_type = node
        .properties
        .get("provider_type")
        .and_then(|pv| serde_json::to_value(pv).ok())
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_default();

    // --- Exchange code for tokens (server-side; secrets never leave here) ---
    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", redirect_uri.as_str()),
        ("client_id", client_id.as_str()),
    ];
    if let Some(secret) = client_secret.as_deref() {
        form.push(("client_secret", secret));
    }
    let resp = reqwest::Client::new()
        .post(&token_url)
        .form(&form)
        .send()
        .await
        .map_err(|e| ApiError::internal(format!("token exchange request failed: {e}")))?;
    if !resp.status().is_success() {
        // Do not echo the provider body — it can contain sensitive material.
        return Err(ApiError::internal(format!(
            "token endpoint returned {}",
            resp.status()
        )));
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|e| ApiError::internal(format!("token endpoint returned bad JSON: {e}")))?;

    let access_token = body
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError::internal("token response missing access_token"))?
        .to_string();
    let refresh_token = body
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let expires_in = body
        .get("expires_in")
        .and_then(|v| v.as_i64())
        .unwrap_or(3600);
    let subject = body
        .get("id_token")
        .and_then(|v| v.as_str())
        .and_then(id_token_email)
        .unwrap_or_default();

    // --- Encrypt the token bundle and append a plaintext-metadata account ---
    let blob = json!({ "access_token": access_token, "refresh_token": refresh_token });
    let tokens_encrypted = SecretBox::new(&master_key)
        .encrypt_json(&blob)
        .map_err(|e| ApiError::internal(format!("failed to encrypt tokens: {e}")))?;

    let account_id = nanoid::nanoid!();
    let account = json!({
        "id": account_id,
        "label": if subject.is_empty() { provider_type.clone() } else { subject.clone() },
        "subject": subject,
        "provider_type": provider_type,
        "expires_at": now_secs() as i64 + expires_in.max(0),
        "tokens_encrypted": tokens_encrypted,
    });

    let mut accounts = connected_accounts(&node);
    accounts.push(account);
    set_connected_accounts(&mut node, accounts)?;
    svc.update_node(node).await?;

    // Relative redirect keeps us origin-agnostic (admin console is same-origin).
    let integration_name = entry
        .integration_path
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_string();
    Ok(Redirect::to(&format!(
        "/{}/integrations?connected={}",
        repo, integration_name
    )))
}

/// Best-effort: pull the `email` claim out of an OIDC `id_token` payload.
///
/// The token arrived directly from the provider over TLS in this same request,
/// so we read the claim without re-verifying the signature — it is used only as
/// a human-facing account label, never for authorization.
fn id_token_email(id_token: &str) -> Option<String> {
    let payload_b64 = id_token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    claims
        .get("email")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}
