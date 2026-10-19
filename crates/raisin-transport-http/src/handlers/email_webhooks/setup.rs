// SPDX-License-Identifier: BSL-1.1

//! Admin routes behind the console's "Delivery tracking" panel.
//!
//! - `POST /api/email/{repo}/webhook-setup/{name}` mints the webhook token (once;
//!   `rotate` replaces it) and, when `register` is set, creates the webhook at
//!   the provider using the sender's own API key.
//! - `GET /api/email/{repo}/webhook-setup/{name}?base=` reports the same state
//!   without changing it.
//!
//! The URL carries the token, so both are admin-only and are the only places it
//! is ever returned. Nothing here returns an API key or a signing secret.

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use raisin_models::auth::AuthContext;
use raisin_models::secret_ref::SecretRef;
use raisin_rocksdb::secret_store::{SecretOwner, SecretScope};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{read_secret, secret_name, sender_entry, store};
use crate::error::ApiError;
use crate::handlers::integrations::{config_branch, require_admin};
use crate::middleware::TenantInfo;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct SetupRequest {
    /// Public origin providers can reach, e.g. `https://db.example.com`.
    pub public_base: String,
    /// Replace an existing token (the old URL stops working).
    #[serde(default)]
    pub rotate: bool,
    /// Create the webhook at the provider with the sender's API key.
    #[serde(default)]
    pub register: bool,
    /// Resend's `whsec_...` secret, pasted by an operator who registered the
    /// webhook by hand. Stored here so it lands in the scope the receiver reads.
    #[serde(default)]
    pub signing_secret: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct StatusQuery {
    pub base: String,
}

fn url_for(base: &str, repo: &str, name: &str, token: &str) -> String {
    format!(
        "{}/api/email/{repo}/webhooks/{name}/{token}",
        base.trim_end_matches('/')
    )
}

fn warning_for(base: &str) -> Option<&'static str> {
    let local = ["localhost", "127.0.0.1", "[::1]", "0.0.0.0"]
        .iter()
        .any(|h| base.contains(h));
    local.then_some(
        "This address is not reachable from the provider; use the public URL of this server.",
    )
}

/// The event names each provider is asked to send.
const BREVO_EVENTS: &[&str] = &[
    "request",
    "delivered",
    "deferred",
    "soft_bounce",
    "hard_bounce",
    "invalid_email",
    "blocked",
    "spam",
    "error",
    "unsubscribed",
    "opened",
    "click",
];
const RESEND_EVENTS: &[&str] = &[
    "email.sent",
    "email.delivered",
    "email.delivery_delayed",
    "email.bounced",
    "email.complained",
    "email.failed",
    "email.suppressed",
    "email.opened",
    "email.clicked",
];

pub async fn status(
    State(state): State<AppState>,
    Path((repo, name)): Path<(String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Query(q): Query<StatusQuery>,
) -> Result<Json<Value>, ApiError> {
    require_admin(auth.as_deref())?;
    let branch = config_branch(&state, &tenant.tenant_id, &repo).await;
    let (kind, _) = sender_entry(&state, &tenant.tenant_id, &repo, &branch, &name)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("no email sender named `{name}`")))?;
    let secrets = store(&state)?;
    let scope = SecretScope::new(&tenant.tenant_id, &repo, &branch);
    let token = read_secret(&secrets, &scope, &secret_name(&name, "token"))?;
    Ok(Json(json!({
        "provider": kind,
        "configured": token.is_some(),
        "url": token.as_deref().map(|t| url_for(&q.base, &repo, &name, t)),
        "signing_secret_set": read_secret(&secrets, &scope, &secret_name(&name, "signing_secret"))?.is_some(),
        "registered": read_secret(&secrets, &scope, &secret_name(&name, "provider_webhook_id"))?.is_some(),
        "needs_signing_secret": kind == "resend",
        "warning": warning_for(&q.base),
    })))
}

pub async fn setup(
    State(state): State<AppState>,
    Path((repo, name)): Path<(String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<SetupRequest>,
) -> Result<Json<Value>, ApiError> {
    require_admin(auth.as_deref())?;
    let branch = config_branch(&state, &tenant.tenant_id, &repo).await;
    let (kind, entry) = sender_entry(&state, &tenant.tenant_id, &repo, &branch, &name)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("no email sender named `{name}`")))?;
    if kind != "resend" && kind != "brevo" {
        return Err(ApiError::new(
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            "NO_FEEDBACK",
            "only Resend and Brevo senders report delivery events",
        ));
    }

    let secrets = store(&state)?;
    let scope = SecretScope::new(&tenant.tenant_id, &repo, &branch);
    let owner = SecretOwner::actor(auth.as_deref().map(|a| a.actor_id()).unwrap_or_default());
    let put = |what: &str, value: &str| -> Result<(), ApiError> {
        secrets
            .put(&scope, &secret_name(&name, what), value.as_bytes(), &owner)
            .map(|_| ())
            .map_err(|e| ApiError::from(raisin_error::Error::from(e)))
    };

    let token = match read_secret(&secrets, &scope, &secret_name(&name, "token"))? {
        Some(t) if !req.rotate => t,
        _ => {
            let t = nanoid::nanoid!(32);
            put("token", &t)?;
            t
        }
    };
    let url = url_for(&req.public_base, &repo, &name, &token);
    if let Some(secret) = req
        .signing_secret
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        put("signing_secret", secret)?;
    }

    let mut registered =
        read_secret(&secrets, &scope, &secret_name(&name, "provider_webhook_id"))?.is_some();
    let mut registration_error: Option<String> = None;
    if req.register && (!registered || req.rotate) {
        let key_ref = entry
            .get("credential_ref")
            .and_then(Value::as_str)
            .unwrap_or("");
        let api_key = SecretRef::parse(key_ref)
            .map_err(|e| e.to_string())
            .and_then(|r| {
                secrets
                    .resolve_string(&scope, &r)
                    .map_err(|e| e.to_string())
            });
        let base = entry
            .get("api_base")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty());
        match api_key {
            Err(e) => {
                registration_error = Some(format!("could not read the sender's API key: {e}"))
            }
            Ok(key) => match register(&kind, base, &key, &url).await {
                Ok((id, signing)) => {
                    put("provider_webhook_id", &id)?;
                    if let Some(s) = signing {
                        put("signing_secret", &s)?;
                    }
                    registered = true;
                }
                Err(e) => registration_error = Some(e),
            },
        }
    }

    Ok(Json(json!({
        "provider": kind,
        "url": url,
        "registered": registered,
        "registration_error": registration_error,
        "signing_secret_set": read_secret(&secrets, &scope, &secret_name(&name, "signing_secret"))?.is_some(),
        "needs_signing_secret": kind == "resend",
        "warning": warning_for(&req.public_base),
    })))
}

/// Create the webhook at the provider. Returns `(provider webhook id, signing secret)`.
async fn register(
    kind: &str,
    api_base: Option<&str>,
    key: &str,
    url: &str,
) -> Result<(String, Option<String>), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let (req, signing) = if kind == "brevo" {
        let base = api_base
            .unwrap_or("https://api.brevo.com/v3")
            .trim_end_matches('/');
        let r = client
            .post(format!("{base}/webhooks"))
            .header("api-key", key)
            .json(&json!({
                "url": url, "description": "RaisinDB delivery tracking",
                "events": BREVO_EVENTS, "type": "transactional",
            }));
        (r, false)
    } else {
        let base = api_base
            .unwrap_or("https://api.resend.com")
            .trim_end_matches('/');
        let r = client
            .post(format!("{base}/webhooks"))
            .bearer_auth(key)
            .json(&json!({
                "endpoint": url, "events": RESEND_EVENTS,
            }));
        (r, true)
    };
    let resp = req
        .send()
        .await
        .map_err(|e| format!("provider unreachable: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let why = body
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("rejected");
        return Err(format!("{kind} answered {status}: {why}"));
    }
    let id = body
        .get("id")
        .map(|v| v.to_string().trim_matches('"').to_string());
    let secret = signing
        .then(|| {
            body.get("signing_secret")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .flatten();
    Ok((id.unwrap_or_else(|| "registered".to_string()), secret))
}
