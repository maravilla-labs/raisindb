// SPDX-License-Identifier: BSL-1.1

//! `GET /api/integrations/{repo}/oauth/callback` — finish an outbound OAuth flow.
//!
//! This endpoint is reached by a browser redirect from the provider, so it
//! carries no bearer token. It is authenticated by the single-use, TTL'd
//! `state` value minted by the authenticated `oauth/start` call.

use axum::{
    extract::{Path, Query, State},
    response::{Html, IntoResponse, Response},
};
use base64::Engine as _;
use raisin_crypto::SecretBox;
use serde::Deserialize;
use serde_json::{json, Value};

use super::state_store::{now_secs, StateKind, OAUTH_STATE_STORE};
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

/// How the connect attempt ended. Both arms leave through [`relay_page`].
enum Outcome {
    Connected { name: String },
    Failed { code: String, description: String },
}

impl Outcome {
    fn failed(code: &str, description: &str) -> Self {
        // Clamp here, once, so no call site can forget: a verbose or hostile
        // provider must not be able to inflate this page or a log line.
        Outcome::Failed {
            code: truncate(code, 64),
            description: truncate(description, 300),
        }
    }
}

/// URL of the connectors page in the admin console, carrying the outcome.
///
/// Used **only** by the no-opener fallback (consent finished in a normal tab).
/// The `/admin` prefix is load-bearing and easy to lose: the console is mounted
/// only on `/admin` and `/admin/{*path}` (`raisin-server`'s `main.rs`) and its
/// router carries `basename="/admin"`, so every other path 404s.
fn console_url(repo: &str, outcome: &Outcome) -> String {
    let path = format!("/admin/{repo}/integrations");
    match outcome {
        Outcome::Connected { name } => format!("{path}?connected={}", urlencoding::encode(name)),
        Outcome::Failed { code, description } => format!(
            "{path}?oauth_error={}&oauth_error_description={}",
            urlencoding::encode(code),
            urlencoding::encode(description),
        ),
    }
}

/// Render the page that terminates the connect flow.
///
/// **Every** outcome after the provider redirect leaves through here, success
/// and failure alike — never an `ApiError`, which painted a raw JSON blob into
/// the popup that the console could not read and the operator could not act on.
///
/// The page is deliberately **self-contained**: it hands the result to the
/// opener via `postMessage` and closes itself. It does not redirect the popup
/// into the console SPA, which booted the entire app inside a 520x680 window
/// purely to relay one message and coupled this flow to the console's route
/// table — exactly what broke when the `/admin` basename was missing, consent
/// landed on a 404, and there was no longer anything left to do the relaying.
///
/// Falls back to navigating to the console when there is no opener, and
/// degrades to a plain link when JavaScript is unavailable.
fn relay_page(repo: &str, outcome: &Outcome) -> Response {
    let payload = match outcome {
        Outcome::Connected { name } => json!({ "type": "raisin-oauth-result", "connected": name }),
        Outcome::Failed { code, description } => json!({
            "type": "raisin-oauth-result",
            "error": code,
            "description": description,
        }),
    };
    let target = console_url(repo, outcome);
    let (heading, detail) = match outcome {
        Outcome::Connected { .. } => (
            "Account connected",
            "You can close this window.".to_string(),
        ),
        Outcome::Failed { code, description } => (
            "Could not connect the account",
            format!("{code}: {description}"),
        ),
    };

    // Both interpolations carry provider-controlled text and are escaped for
    // their context: `json_for_script` neutralises `<` so a description
    // containing `</script>` cannot break out of the block, and `escape_html`
    // covers the visible body.
    let html = format!(
        r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>{heading}</title>
<style>
 body{{font:14px/1.5 system-ui,sans-serif;margin:0;padding:2rem;background:#18181b;color:#e4e4e7}}
 .detail{{font-family:ui-monospace,monospace;font-size:12px;word-break:break-word}}
 a{{color:#7dd3fc}}
</style></head>
<body>
<h1 style="font-size:1.1rem">{heading}</h1>
<p class="detail">{detail_html}</p>
<p><a href="{target_html}">Return to the connectors page</a></p>
<script>
(function () {{
  var result = {payload};
  try {{
    if (window.opener && !window.opener.closed) {{
      window.opener.postMessage(result, window.location.origin);
      window.close();
      return;
    }}
  }} catch (e) {{ /* opener gone: fall through to the navigation below */ }}
  window.location.replace({target_js});
}})();
</script>
</body></html>"#,
        heading = heading,
        detail_html = escape_html(&detail),
        target_html = escape_html(&target),
        target_js = json_for_script(&Value::String(target.clone())),
        payload = json_for_script(&payload),
    );

    Html(html).into_response()
}

/// Serialise JSON for embedding inside a `<script>` block.
///
/// `<` becomes `<` so that no interpolated string — the provider's
/// `error_description` above all — can emit a literal `</script>` and escape the
/// block. `>` and `&` follow for defence in depth.
fn json_for_script(value: &Value) -> String {
    value
        .to_string()
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
}

/// Minimal entity escaping for text interpolated into the page body.
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// Exchange the authorization code for tokens, persist an encrypted account
/// entry, and redirect back to the admin console.
pub async fn callback(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    Query(q): Query<CallbackQuery>,
) -> Result<Response, ApiError> {
    // A provider-side refusal is a normal outcome of a user-facing flow, not a
    // server fault. Returning an ApiError rendered a raw JSON blob inside the
    // popup; redirect back to the console instead so the operator sees the
    // provider's own code (e.g. AADSTS50194) in context.
    if let Some(err) = q.error {
        return Ok(relay_page(
            &repo,
            &Outcome::failed(&err, q.error_description.as_deref().unwrap_or("")),
        ));
    }
    let (Some(code), Some(state_token)) = (q.code, q.state) else {
        return Ok(relay_page(
            &repo,
            &Outcome::failed(
                "invalid_request",
                "The provider redirected back without an authorization code or state parameter.",
            ),
        ));
    };

    let Some(entry) = OAUTH_STATE_STORE.consume(&state_token, StateKind::Integration, now_secs())
    else {
        return Ok(relay_page(&repo, &Outcome::failed(
            "invalid_state",
            "This connect attempt is unknown or expired (it is valid for 10 minutes and single-use). \
             Start the connect again. On a multi-node deployment, the start and callback must also \
             land on the same node — the state store is in-process.",
        )));
    };
    // Defend against a state minted for a different repo being replayed here.
    if entry.repo != repo {
        return Ok(relay_page(
            &repo,
            &Outcome::failed(
                "invalid_state",
                "This connect attempt was started for a different repository.",
            ),
        ));
    }

    let master_key = state.get_master_key()?;
    let svc = super::config_service(&state, &entry.tenant_id, &entry.repo, "integration-oauth");
    let Some(mut node) = svc.get_by_path(&entry.target_path).await? else {
        return Ok(relay_page(
            &repo,
            &Outcome::failed(
                "unknown_connector",
                &format!(
                "The connector {} no longer exists — it was deleted or renamed during the connect.",
                entry.target_path
            ),
            ),
        ));
    };

    let cfg = oauth_config(&node);
    let (Some(token_url), Some(client_id)) =
        (json_str(&cfg, "token_url"), json_str(&cfg, "client_id"))
    else {
        return Ok(relay_page(
            &repo,
            &Outcome::failed(
                "incomplete_config",
                "This connector is missing its OAuth Token URL or Client ID. Fill both in, click \
             Update to save, then connect again.",
            ),
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
            return Ok(relay_page(
                &repo,
                &Outcome::failed(
                    "token_endpoint_unreachable",
                    &format!("Could not reach the token endpoint {token_url}."),
                ),
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
            integration_path = %entry.target_path,
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
        return Ok(relay_page(&repo, &Outcome::failed(&code, &desc)));
    }
    let body: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, %token_url, "token endpoint returned a non-JSON body");
            return Ok(relay_page(
                &repo,
                &Outcome::failed(
                    "bad_token_response",
                    "The token endpoint returned a response that is not valid JSON.",
                ),
            ));
        }
    };

    let Some(access_token) = body
        .get("access_token")
        .and_then(|v| v.as_str())
        .map(String::from)
    else {
        return Ok(relay_page(
            &repo,
            &Outcome::failed(
                "bad_token_response",
                "The token endpoint accepted the exchange but returned no access_token.",
            ),
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
    // What the provider ACTUALLY granted, which is not always what was asked
    // for. Recorded so the console can tell an account that is missing a
    // permission from one that is merely broken — the difference between
    // "reconnect to grant it" and "something else is wrong". Both Microsoft and
    // Google return a space-delimited `scope`; an absent one means the provider
    // did not say, which is not the same as "none" and is stored as such.
    let granted_scopes: Vec<String> = body
        .get("scope")
        .and_then(|v| v.as_str())
        .map(|s| s.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default();

    // --- Encrypt the token bundle and append a plaintext-metadata account ---
    // Past this point the user has already consented and the provider has
    // already issued tokens, so a failure here loses a completed grant. Say so
    // plainly rather than painting a JSON blob into the popup.
    let blob = json!({ "access_token": access_token, "refresh_token": refresh_token });
    let tokens_encrypted =
        match SecretBox::new(&master_key).encrypt_json(&blob) {
            Ok(t) => t,
            Err(e) => {
                tracing::error!(error = %e, "failed to encrypt OAuth tokens for storage");
                return Ok(relay_page(&repo, &Outcome::failed(
                "storage_failed",
                "Authorization succeeded but the tokens could not be encrypted for storage.",
            )));
            }
        };

    let mut accounts = connected_accounts(&node);

    // RECONNECT, not "connect a second time".
    //
    // An account is identified by the provider's own subject, so re-consenting
    // as the same user UPDATES that account in place and keeps its id. This is
    // the whole mechanism behind widening a scope: every mount references an
    // account by `account_ref`, so minting a new id would leave those mounts
    // pointing at the OLD account and its old, narrower grant — the operator
    // consents to the new permission, sees a second account appear, and nothing
    // whatsoever changes about the mounts they were trying to fix.
    //
    // That is what forced disconnect-then-connect, which is worse than it
    // sounds: disconnecting is precisely what breaks the `account_ref` the
    // mounts depend on. Matching on subject removes the need for either step.
    //
    // An empty subject cannot be matched on — some providers return no id_token
    // — so those still append. Guessing which existing account an anonymous
    // grant belongs to would eventually attach someone's tokens to someone
    // else's account.
    let existing = if subject.is_empty() {
        None
    } else {
        accounts.iter().position(|a| {
            a.get("subject").and_then(|v| v.as_str()) == Some(subject.as_str())
                && a.get("provider_type").and_then(|v| v.as_str()) == Some(provider_type.as_str())
        })
    };

    let account_id = match existing {
        Some(i) => accounts[i]
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        None => nanoid::nanoid!(),
    };
    let account = json!({
        "id": account_id,
        "label": if subject.is_empty() { provider_type.clone() } else { subject.clone() },
        "subject": subject,
        "provider_type": provider_type,
        "expires_at": now_secs() as i64 + expires_in.max(0),
        "tokens_encrypted": tokens_encrypted,
        "scopes": granted_scopes,
        "connected_at": now_secs() as i64,
    });

    match existing {
        Some(i) => {
            tracing::info!(
                integration_path = %entry.target_path,
                account_id = %account_id,
                "re-authorized an existing account in place; its id and every mount                  referencing it are unchanged"
            );
            accounts[i] = account;
        }
        None => accounts.push(account),
    }
    set_connected_accounts(&mut node, accounts)?;
    if let Err(e) = svc.update_node(node).await {
        tracing::error!(error = %e, integration_path = %entry.target_path,
            "failed to persist the connected account after a successful OAuth exchange");
        return Ok(relay_page(
            &repo,
            &Outcome::failed(
                "storage_failed",
                "Authorization succeeded but the connected account could not be saved.",
            ),
        ));
    }

    let integration_name = entry
        .target_path
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_string();
    Ok(relay_page(
        &repo,
        &Outcome::Connected {
            name: integration_name,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rebuilds what `relay_page` renders; `Response` bodies are streams, so the
    /// HTML is produced here rather than read back out of one.
    fn page(outcome: &Outcome) -> String {
        let target = console_url("studio", outcome);
        let payload = match outcome {
            Outcome::Connected { name } => {
                json!({ "type": "raisin-oauth-result", "connected": name })
            }
            Outcome::Failed { code, description } => json!({
                "type": "raisin-oauth-result", "error": code, "description": description,
            }),
        };
        format!(
            "{} {} {}",
            json_for_script(&payload),
            escape_html(&target),
            json_for_script(&Value::String(target)),
        )
    }

    /// The `/admin` prefix is the whole point of `console_url`: the console is
    /// mounted nowhere else, so the fallback navigation 404s without it. This
    /// shipped broken on both the success and the failure path.
    #[test]
    fn fallback_url_targets_the_admin_console_mount() {
        let connected = Outcome::Connected {
            name: "ms-graph".into(),
        };
        assert_eq!(
            console_url("studio", &connected),
            "/admin/studio/integrations?connected=ms-graph"
        );
        assert!(
            console_url("studio", &Outcome::failed("invalid_client", "x"))
                .starts_with("/admin/studio/integrations?oauth_error=invalid_client")
        );
    }

    /// A provider description containing `</script>` must not be able to close
    /// the block and inject markup — this text comes from outside RaisinDB.
    #[test]
    fn script_payload_cannot_break_out_of_the_block() {
        let evil = Outcome::failed(
            "invalid_client",
            "</script><img src=x onerror=alert(1)>oops",
        );
        let rendered = page(&evil);
        assert!(!rendered.contains("</script>"));
        assert!(!rendered.contains("<img"));
        assert!(rendered.contains("\\u003c/script\\u003e"));
    }

    /// The same text is also interpolated into the visible body.
    #[test]
    fn body_text_is_entity_escaped() {
        assert_eq!(
            escape_html("<b>&\"'</b>"),
            "&lt;b&gt;&amp;&quot;&#x27;&lt;/b&gt;"
        );
    }

    /// Microsoft's descriptions carry spaces, colons and quotes, and land in a
    /// query string on the fallback path.
    #[test]
    fn fallback_url_encodes_provider_text() {
        let url = console_url(
            "studio",
            &Outcome::failed("invalid_client", "AADSTS7000215: bad 'secret'"),
        );
        assert!(url.contains("AADSTS7000215%3A%20bad%20%27secret%27"));
        assert!(!url.contains(' '));
    }

    #[test]
    fn failure_text_is_clamped() {
        let Outcome::Failed { code, description } =
            Outcome::failed(&"c".repeat(500), &"d".repeat(5000))
        else {
            panic!("expected a failure outcome")
        };
        assert_eq!(code.chars().count(), 64);
        assert_eq!(description.chars().count(), 300);
    }

    /// Clamping is char-based, so a multi-byte description cannot be split
    /// mid-codepoint (which would panic on a byte slice).
    #[test]
    fn clamp_never_splits_utf8() {
        assert_eq!(truncate(&"é".repeat(500), 300).chars().count(), 300);
    }
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
