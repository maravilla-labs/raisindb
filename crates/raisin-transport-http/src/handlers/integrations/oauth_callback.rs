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

/// Send the browser back to the console with a diagnosable failure.
///
/// **Every** failure after the provider redirect must come back this way, not as
/// an `ApiError`. This endpoint renders in a popup opened by the console: an
/// `ApiError` paints a raw JSON blob there, which the console cannot read and
/// the operator cannot act on — and the popup is closed moments later by the
/// parent's poll, so the blob is usually never even seen. The console picks
/// `oauth_error` / `oauth_error_description` off the URL and shows a toast.
fn error_redirect(repo: &str, code: &str, description: &str) -> Redirect {
    Redirect::to(&format!(
        "/{repo}/integrations?oauth_error={}&oauth_error_description={}",
        urlencoding::encode(&truncate(code, 64)),
        urlencoding::encode(&truncate(description, 300)),
    ))
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
        return Ok(error_redirect(
            &repo,
            &err,
            q.error_description.as_deref().unwrap_or(""),
        ));
    }
    let (Some(code), Some(state_token)) = (q.code, q.state) else {
        return Ok(error_redirect(
            &repo,
            "invalid_request",
            "The provider redirected back without an authorization code or state parameter.",
        ));
    };

    let Some(entry) = OAUTH_STATE_STORE.consume(&state_token, now_secs()) else {
        return Ok(error_redirect(
            &repo,
            "invalid_state",
            "This connect attempt is unknown or expired (it is valid for 10 minutes and single-use). \
             Start the connect again. On a multi-node deployment, the start and callback must also \
             land on the same node — the state store is in-process.",
        ));
    };
    // Defend against a state minted for a different repo being replayed here.
    if entry.repo != repo {
        return Ok(error_redirect(
            &repo,
            "invalid_state",
            "This connect attempt was started for a different repository.",
        ));
    }

    let master_key = state.get_master_key()?;
    let svc = super::config_service(&state, &entry.tenant_id, &entry.repo, "integration-oauth");
    let Some(mut node) = svc.get_by_path(&entry.integration_path).await? else {
        return Ok(error_redirect(
            &repo,
            "unknown_connector",
            &format!(
                "The connector {} no longer exists — it was deleted or renamed during the connect.",
                entry.integration_path
            ),
        ));
    };

    let cfg = oauth_config(&node);
    let (Some(token_url), Some(client_id)) =
        (json_str(&cfg, "token_url"), json_str(&cfg, "client_id"))
    else {
        return Ok(error_redirect(
            &repo,
            "incomplete_config",
            "This connector is missing its OAuth Token URL or Client ID. Fill both in, click \
             Update to save, then connect again.",
        ));
    };
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
    let resp = match reqwest::Client::new()
        .post(&token_url)
        .form(&form)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, %token_url, "OAuth token exchange could not reach the provider");
            return Ok(error_redirect(
                &repo,
                "token_endpoint_unreachable",
                &format!("Could not reach the token endpoint {token_url}."),
            ));
        }
    };
    if !resp.status().is_success() {
        // The provider's own error code is the ONLY thing that distinguishes the
        // several failure modes behind a bare `401` (Microsoft: a wrong secret,
        // an unsent secret, and a redirect URI registered under the wrong app
        // platform are all `invalid_client`). Surface the spec-defined
        // `error` / `error_description` and nothing else — the rest of the body
        // is discarded rather than echoed.
        let status = resp.status();
        let (code, desc) = super::oauth_error_parts(&resp.text().await.unwrap_or_default());
        tracing::warn!(
            %status,
            integration_path = %entry.integration_path,
            %redirect_uri,
            client_secret_sent = client_secret.is_some(),
            provider_error = %code,
            provider_error_description = %desc,
            "OAuth token exchange rejected by the provider"
        );
        // `client_secret_sent` is the one piece of context the provider cannot
        // give the operator, and it separates "wrong secret" from "no secret
        // reached the provider at all" — the failure mode of a secret that was
        // typed but never saved, or stored under a different RAISIN_MASTER_KEY.
        let hint = if client_secret.is_none() {
            " (RaisinDB sent no client secret — none is stored for this connector, or the stored \
             one could not be decrypted; re-enter it and click Update before connecting)"
        } else {
            ""
        };
        let code = if code.is_empty() {
            "token_exchange_failed".to_string()
        } else {
            code
        };
        let desc = if desc.is_empty() {
            format!("The token endpoint returned HTTP {status}.{hint}")
        } else {
            format!("{desc}{hint}")
        };
        return Ok(error_redirect(&repo, &code, &desc));
    }
    let body: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, %token_url, "token endpoint returned a non-JSON body");
            return Ok(error_redirect(
                &repo,
                "bad_token_response",
                "The token endpoint returned a response that is not valid JSON.",
            ));
        }
    };

    let Some(access_token) = body
        .get("access_token")
        .and_then(|v| v.as_str())
        .map(String::from)
    else {
        return Ok(error_redirect(
            &repo,
            "bad_token_response",
            "The token endpoint accepted the exchange but returned no access_token.",
        ));
    };
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
    // Past this point the user has already consented and the provider has
    // already issued tokens, so a failure here loses a completed grant. Say so
    // plainly rather than painting a JSON blob into the popup.
    let blob = json!({ "access_token": access_token, "refresh_token": refresh_token });
    let tokens_encrypted = match SecretBox::new(&master_key).encrypt_json(&blob) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "failed to encrypt OAuth tokens for storage");
            return Ok(error_redirect(
                &repo,
                "storage_failed",
                "Authorization succeeded but the tokens could not be encrypted for storage.",
            ));
        }
    };

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
    if let Err(e) = svc.update_node(node).await {
        tracing::error!(error = %e, integration_path = %entry.integration_path,
            "failed to persist the connected account after a successful OAuth exchange");
        return Ok(error_redirect(
            &repo,
            "storage_failed",
            "Authorization succeeded but the connected account could not be saved.",
        ));
    }

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
