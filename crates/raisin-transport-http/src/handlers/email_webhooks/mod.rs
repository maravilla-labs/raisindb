// SPDX-License-Identifier: BSL-1.1

//! Email delivery feedback: the provider-facing webhook receiver and the
//! admin-facing setup routes that make it self-service.
//!
//! `raisin.email.send` proves only that a provider ACCEPTED a message. Brevo and
//! Resend report what happened next (delivered, deferred, bounced, blocked,
//! complained) by calling a webhook. This module is that endpoint:
//!
//! - [`receive`] — public, `POST /api/email/{repo}/webhooks/{name}/{token}`.
//!   `{name}` is the sender entry in `/config/email`, which decides the payload
//!   format. The unguessable `{token}` is the guard for Brevo (which does not
//!   sign payloads); Resend additionally has its Svix signature verified over
//!   the RAW body, which is why this is a native route and not a function
//!   trigger — the generic webhook route parses the body before a function sees
//!   it, and the signature is over the bytes.
//! - [`setup`] / [`status`] — admin-gated; mint the token, optionally register
//!   the webhook with the provider using the sender's own API key, and report
//!   the URL to paste when auto-registration is not possible.
//!
//! The token and the signing secret live in the secret store, under
//! `email/webhook/{name}/...`, never in a node property.

mod receive;
mod setup;

pub use receive::receive;
pub use setup::{setup, status};

use raisin_models::auth::AuthContext;
use raisin_rocksdb::secret_store::{SecretError, SecretScope, SecretStore};
use serde_json::Value;

use crate::error::ApiError;
use crate::state::AppState;

/// Workspace and path of the tenant's `raisin:EmailConfig`.
pub(super) const CONFIG_WORKSPACE: &str = "raisin:system";
const EMAIL_CONFIG_PATH: &str = "/config/email";

/// The store name of one of a sender's webhook secrets.
pub(super) fn secret_name(sender: &str, what: &str) -> String {
    format!("email/webhook/{sender}/{what}")
}

pub(super) fn store(state: &AppState) -> Result<std::sync::Arc<SecretStore>, ApiError> {
    Ok(state.storage().secret_store()?)
}

/// Read a secret as text. Absent and deleted are both `None`: for a webhook, "no
/// token configured" and "token deleted" mean the same thing — nothing can match.
pub(super) fn read_secret(
    store: &SecretStore,
    scope: &SecretScope,
    name: &str,
) -> Result<Option<String>, ApiError> {
    match store.get(scope, name, None) {
        Ok(bytes) => Ok(String::from_utf8(bytes).ok()),
        Err(SecretError::Pending { .. }) | Err(SecretError::Gone { .. }) => Ok(None),
        Err(e) => Err(ApiError::from(raisin_error::Error::from(e))),
    }
}

/// A node service acting as the platform, not as the (unauthenticated) caller.
pub(super) fn system_service(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
) -> raisin_core::NodeService<crate::state::Store> {
    let mut auth = AuthContext::system();
    auth.user_id = Some("email-webhook".to_string());
    state.node_service_for_context(tenant_id, repo, branch, CONFIG_WORKSPACE, Some(auth))
}

/// The sender entry called `name` from `/config/email`: `(provider kind, entry)`.
/// `None` when there is no config or no such sender; the caller decides how that
/// is reported (the public receiver must not distinguish it from a bad token).
pub(super) async fn sender_entry(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    name: &str,
) -> Result<Option<(String, Value)>, ApiError> {
    let svc = system_service(state, tenant_id, repo, branch);
    let Some(node) = svc.get_by_path(EMAIL_CONFIG_PATH).await? else {
        return Ok(None);
    };
    let props = serde_json::to_value(&node.properties).unwrap_or(Value::Null);
    let Some(entry) = props
        .get("providers")
        .and_then(Value::as_array)
        .and_then(|a| {
            a.iter()
                .find(|e| e.get("name").and_then(Value::as_str) == Some(name))
        })
        .cloned()
    else {
        return Ok(None);
    };
    let kind = entry
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Ok(Some((kind, entry)))
}

/// Constant-time byte equality (no `subtle` dependency in this crate).
pub(super) fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}
