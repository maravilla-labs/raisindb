// SPDX-License-Identifier: BSL-1.1

//! `GET /api/integrations/{repo}/setup-urls` and
//! `GET /api/integrations/{repo}/mounts/{mount_id}/setup-urls` — hand the admin
//! UI the exact provider-side URLs a user must register when wiring an external
//! system (Experimental).
//!
//! Two URLs the user pastes on the provider side:
//! - **OAuth redirect URI** (`.../oauth/callback`) — the "redirect URI" / "reply
//!   URL" of their OAuth client. Does not depend on a mount, so the
//!   integration-level route returns it before any mount exists.
//! - **Notification URL** (`.../notifications/{mount_token}`) — the per-mount push
//!   endpoint some providers (e.g. Gmail Pub/Sub) require the operator to paste
//!   into their push subscription. The mount route lazily mints and persists the
//!   stable `push_mount_token` so this URL is stable and the public notifications
//!   endpoint can resolve it.
//!
//! Base URL resolution mirrors the rest of the server (`RAISINDB_BASE_URL`). When
//! unset we return the paths with a literal `{base}` placeholder and
//! `base_url_configured: false` rather than failing — the UI shows the user which
//! host portion they still need to configure.

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use serde::Serialize;
use serde_json::Value;

use super::require_admin;
use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::AppState;

/// Substituted for the host portion in every URL when `RAISINDB_BASE_URL` is
/// unset, so the UI can render the path and flag the missing configuration.
const BASE_PLACEHOLDER: &str = "{base}";

/// Length of the random suffix of a `push_mount_token`. MUST match the engine's
/// `ensure_mount_token` (`nanoid::nanoid!(32)`) so the HTTP and engine paths mint
/// identical tokens and never diverge.
const MOUNT_TOKEN_NANOID_LEN: usize = 32;

/// The provider-side URLs the admin UI displays for a connector/mount.
#[derive(Debug, Serialize)]
pub struct SetupUrlsResponse {
    /// The OAuth "redirect URI" / "reply URL" to register on the provider's
    /// OAuth client. Independent of any mount.
    pub oauth_redirect_uri: String,
    /// The per-mount push notification URL, or `None` for the integration-level
    /// variant (no mount in scope).
    pub notification_url: Option<String>,
    /// The stable per-mount token embedded in `notification_url`, or `None` when
    /// no mount is in scope.
    pub mount_token: Option<String>,
    /// `false` when `RAISINDB_BASE_URL` is unset: the URLs carry a `{base}`
    /// placeholder the user must replace with the server's public origin.
    pub base_url_configured: bool,
}

/// Integration-level variant: the OAuth redirect URI only (no mount needed).
///
/// Lets the connector page show the redirect URI to register before the user has
/// created any mount.
pub async fn setup_urls_integration(
    State(_state): State<AppState>,
    Path(repo): Path<String>,
    Extension(_tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<SetupUrlsResponse>, ApiError> {
    require_admin(auth.as_deref())?;
    let (base, configured) = base_or_placeholder();
    Ok(Json(SetupUrlsResponse {
        oauth_redirect_uri: oauth_redirect_uri(&base, &repo),
        notification_url: None,
        mount_token: None,
        base_url_configured: configured,
    }))
}

/// Mount-level variant: the OAuth redirect URI plus the per-mount notification
/// URL. Lazily mints and persists a stable `push_mount_token` on first call.
pub async fn setup_urls_mount(
    State(state): State<AppState>,
    Path((repo, mount_id)): Path<(String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<SetupUrlsResponse>, ApiError> {
    require_admin(auth.as_deref())?;
    let (base, configured) = base_or_placeholder();

    let svc = super::config_service(&state, &tenant.tenant_id, &repo, "integration-setup");
    let mut node = svc
        .get(&mount_id)
        .await?
        .ok_or_else(|| ApiError::node_not_found(mount_id.clone()))?;

    // Read the engine-managed `state` object as raw JSON and lazily ensure the
    // token (+ backfill the notification URL). Working in `Value` keeps this
    // crate free of the engine's `MountState` type while writing the exact same
    // fields the engine and notifications endpoint read.
    let mut state_json = node
        .properties
        .get("state")
        .and_then(|pv| serde_json::to_value(pv).ok())
        .unwrap_or(Value::Null);

    let (token, changed) =
        ensure_token_in_state(&mount_id, &repo, &base, configured, &mut state_json);

    if changed {
        let pv: PropertyValue = serde_json::from_value(state_json)
            .map_err(|e| ApiError::internal(format!("failed to encode mount state: {e}")))?;
        node.properties.insert("state".to_string(), pv);
        svc.update_node(node).await?;
    }

    Ok(Json(SetupUrlsResponse {
        oauth_redirect_uri: oauth_redirect_uri(&base, &repo),
        notification_url: Some(notification_url(&base, &repo, &token)),
        mount_token: Some(token),
        base_url_configured: configured,
    }))
}

// --- Pure helpers (unit-tested below; no storage or env access) ---

/// Build the OAuth redirect URI (frozen shape — this is what the provider calls
/// back and what `oauth_callback` is routed on).
pub(crate) fn oauth_redirect_uri(base: &str, repo: &str) -> String {
    format!("{base}/api/integrations/{repo}/oauth/callback")
}

/// The one path `oauth_callback` is actually routed on, for a given repo.
///
/// `routes/integrations.rs` registers exactly one GET under `oauth/`; any other
/// path 404s. Kept next to [`oauth_redirect_uri`] so the two cannot drift.
pub(crate) fn oauth_callback_path(repo: &str) -> String {
    format!("/api/integrations/{repo}/oauth/callback")
}

/// Build the per-mount push notification URL (frozen shape — the notifications
/// endpoint parses the token from the last path segment).
fn notification_url(base: &str, repo: &str, mount_token: &str) -> String {
    format!("{base}/api/integrations/{repo}/notifications/{mount_token}")
}

/// Ensure `state` carries a stable `push_mount_token`, minting one on first call
/// in the engine's exact `"{mount_id}.{nanoid}"` format. When a real base URL is
/// available and no `push_notification_url` is stored yet, backfill it (so the
/// public notifications endpoint — which matches on that URL's last segment —
/// resolves this token). Never clobbers an engine-set URL. Returns
/// `(token, changed)`; `changed` is `false` on a repeat call, making it
/// idempotent.
fn ensure_token_in_state(
    mount_id: &str,
    repo: &str,
    base: &str,
    base_configured: bool,
    state: &mut Value,
) -> (String, bool) {
    if !state.is_object() {
        *state = Value::Object(serde_json::Map::new());
    }
    let obj = state.as_object_mut().expect("state coerced to object");

    let mut changed = false;
    let token = match obj
        .get("push_mount_token")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        Some(t) => t.to_string(),
        None => {
            let t = format!("{mount_id}.{}", nanoid::nanoid!(MOUNT_TOKEN_NANOID_LEN));
            obj.insert("push_mount_token".to_string(), Value::String(t.clone()));
            changed = true;
            t
        }
    };

    let has_url = obj
        .get("push_notification_url")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());
    if base_configured && !has_url {
        obj.insert(
            "push_notification_url".to_string(),
            Value::String(notification_url(base, repo, &token)),
        );
        changed = true;
    }

    (token, changed)
}

/// The configured canonical base URL (`RAISINDB_BASE_URL`, trailing slash
/// trimmed), or `None` when unset/empty. Mirrors the engine and OAuth server.
fn configured_base_url() -> Option<String> {
    std::env::var("RAISINDB_BASE_URL")
        .ok()
        .map(|b| b.trim_end_matches('/').to_string())
        .filter(|b| !b.is_empty())
}

/// Resolve the base for URL construction: `(base, configured)`. Falls back to the
/// `{base}` placeholder (never fails) so the UI still gets a usable path.
pub(crate) fn base_or_placeholder() -> (String, bool) {
    match configured_base_url() {
        Some(b) => (b, true),
        None => (BASE_PLACEHOLDER.to_string(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn urls_use_configured_base() {
        let base = "https://db.example.com";
        assert_eq!(
            oauth_redirect_uri(base, "acme"),
            "https://db.example.com/api/integrations/acme/oauth/callback"
        );
        assert_eq!(
            notification_url(base, "acme", "m1.tok"),
            "https://db.example.com/api/integrations/acme/notifications/m1.tok"
        );
    }

    #[test]
    fn urls_use_placeholder_when_base_unset() {
        // The placeholder path is still well-formed; only the host is symbolic.
        assert_eq!(
            oauth_redirect_uri(BASE_PLACEHOLDER, "acme"),
            "{base}/api/integrations/acme/oauth/callback"
        );
        assert_eq!(
            notification_url(BASE_PLACEHOLDER, "acme", "m1.tok"),
            "{base}/api/integrations/acme/notifications/m1.tok"
        );
    }

    #[test]
    fn token_is_minted_once_and_is_idempotent() {
        let base = "https://db.example.com";
        let mut state = Value::Null;

        let (tok1, changed1) = ensure_token_in_state("mount-1", "acme", base, true, &mut state);
        assert!(changed1, "first call must persist a new token");
        assert!(tok1.starts_with("mount-1."), "engine token format");
        assert_eq!(
            tok1.len(),
            "mount-1.".len() + MOUNT_TOKEN_NANOID_LEN,
            "suffix length must match the engine's nanoid length"
        );
        // The notification URL was backfilled into state on the first call.
        assert_eq!(
            state.get("push_notification_url").and_then(|v| v.as_str()),
            Some(notification_url(base, "acme", &tok1).as_str())
        );

        // Second call: same token, no further mutation.
        let (tok2, changed2) = ensure_token_in_state("mount-1", "acme", base, true, &mut state);
        assert_eq!(tok1, tok2, "repeat call must return the same token");
        assert!(!changed2, "repeat call must not mutate state");
    }

    #[test]
    fn token_persisted_without_url_when_base_unset() {
        let mut state = Value::Null;
        let (tok, changed) =
            ensure_token_in_state("mount-2", "acme", BASE_PLACEHOLDER, false, &mut state);
        assert!(changed);
        assert!(tok.starts_with("mount-2."));
        // No real base => do not persist a placeholder notification URL.
        assert!(state.get("push_notification_url").is_none());

        // A later call once the base is configured backfills the URL (still the
        // same token) and reports the mutation.
        let base = "https://db.example.com";
        let (tok2, changed2) = ensure_token_in_state("mount-2", "acme", base, true, &mut state);
        assert_eq!(tok, tok2);
        assert!(changed2);
        assert_eq!(
            state.get("push_notification_url").and_then(|v| v.as_str()),
            Some(notification_url(base, "acme", &tok).as_str())
        );
    }

    #[test]
    fn engine_set_notification_url_is_not_clobbered() {
        let existing = json!({
            "push_mount_token": "mount-3.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "push_notification_url": "https://old.example.com/api/integrations/acme/notifications/mount-3.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        });
        let mut state = existing.clone();
        let (tok, changed) = ensure_token_in_state(
            "mount-3",
            "acme",
            "https://new.example.com",
            true,
            &mut state,
        );
        assert_eq!(tok, "mount-3.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert!(!changed, "existing token + URL must be left untouched");
        assert_eq!(state, existing);
    }
}
