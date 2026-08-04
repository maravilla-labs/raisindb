// SPDX-License-Identifier: BSL-1.1

//! Outbound OAuth + mount admin handlers ("connect Google", token lifecycle,
//! manual sync).
//!
//! These endpoints configure external-system integrations. They are distinct
//! from `handlers::oauth_as`, which is the *inbound* OAuth 2.1 authorization
//! server for MCP clients. Everything here lives under `/api/integrations/...`.
//!
//! Security invariants enforced throughout this module:
//! - `client_secret` and `refresh_token` never appear in a response body or a
//!   log line. Tokens are stored only as `tokens_encrypted` (AES-256-GCM).
//! - Config mutations are admin-only. The one exception is the OAuth
//!   `callback`, which is a browser redirect from the provider and therefore
//!   cannot carry a bearer token — it is authenticated instead by the single-use,
//!   TTL'd `state` parameter minted by the authenticated `start` call.

mod accounts_lock;
mod adapter_invoke;
mod browse;
mod client_secret;
mod config_secrets;
mod connections;
mod manage;
mod notifications;
mod oauth_callback;
mod oauth_start;
pub(crate) mod setup_info;
pub mod state_store;
mod test_connection;

#[cfg(feature = "storage-rocksdb")]
pub use browse::browse;
pub use client_secret::set_client_secret;
pub use config_secrets::set_config_secrets;
pub use connections::{create_connection, delete_connection, list_connections, update_connection};
pub use manage::{disconnect, sync_mount};
pub use notifications::notify;
pub use oauth_callback::callback;
pub use oauth_start::start;
pub use setup_info::{setup_urls_integration, setup_urls_mount};
pub use test_connection::test_connection;

use base64::Engine as _;
use raisin_crypto::SecretBox;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use serde_json::Value;

use crate::error::ApiError;
use crate::state::{AppState, Store};

/// Branch that holds integration configuration.
const CONFIG_BRANCH: &str = "main";
/// Workspace that holds integration/mount configuration in every repo.
const CONFIG_WORKSPACE: &str = "raisin:system";

/// Reject the request unless the caller is an admin.
///
/// Accepts the system context or any principal carrying the `system_admin` /
/// `admin` role. Mutating integration config is an admin operation.
pub(crate) fn require_admin(auth: Option<&AuthContext>) -> Result<(), ApiError> {
    let ok = auth.is_some_and(|a| {
        a.is_system
            || a.roles.iter().any(|r| r == "system_admin" || r == "admin")
            || a.permissions().is_some_and(|p| p.is_system_admin)
    });
    if ok {
        Ok(())
    } else {
        Err(ApiError::new(
            axum::http::StatusCode::FORBIDDEN,
            "FORBIDDEN",
            "Integration configuration requires an admin principal",
        ))
    }
}

/// Build a `raisin:system`-scoped, system-auth `NodeService` for integration
/// config reads/writes. System context is used because config nodes are
/// admin-managed and the callback path has no authenticated principal.
pub(crate) fn config_service(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    actor: &str,
) -> raisin_core::NodeService<Store> {
    let mut auth = AuthContext::system();
    auth.user_id = Some(actor.to_string());
    state.node_service_for_context(tenant_id, repo, CONFIG_BRANCH, CONFIG_WORKSPACE, Some(auth))
}

/// An object-valued property of a node as raw JSON (or `Null`).
pub(crate) fn json_prop(node: &Node, key: &str) -> Value {
    node.properties
        .get(key)
        .and_then(|pv| serde_json::to_value(pv).ok())
        .unwrap_or(Value::Null)
}

/// The `oauth_config` object of an integration node as raw JSON (or `Null`).
pub(crate) fn oauth_config(node: &Node) -> Value {
    json_prop(node, "oauth_config")
}

/// Read a string field from a JSON object, trimming empties to `None`.
pub(crate) fn json_str(obj: &Value, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Read the scope list from an `oauth_config`, accepting either a JSON array of
/// strings or a single space-delimited string. Returns the space-joined scope
/// string OAuth expects.
pub(crate) fn scopes_string(cfg: &Value) -> String {
    match cfg.get("scopes") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(" "),
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

/// Resolve the OAuth `redirect_uri` for a repo, repairing a misconfigured one.
///
/// The provider redirects the browser to whatever we send here, and
/// `oauth_callback` is routed on exactly one path
/// ([`setup_info::oauth_callback_path`]). A stored value pointing anywhere else
/// — the shipped templates once hinted at a URL missing the `{repo}` segment —
/// sends the user to a 404 *after* they have consented, which is both confusing
/// and unfixable from the provider's side.
///
/// Resolution order:
/// 1. Empty/absent → the canonical URI.
/// 2. Stored value whose path is already canonical → used verbatim (its host may
///    legitimately differ from `RAISINDB_BASE_URL`: proxy, custom domain).
/// 3. Stored value with a wrong path → its scheme+host are kept and the path is
///    corrected. This repairs the exact production misconfiguration in place.
/// 4. Unparseable → the canonical URI.
///
/// Both `oauth/start` and `oauth/callback` must call this, because the provider
/// requires the two to match byte-for-byte.
pub(crate) fn resolve_redirect_uri(cfg: &Value, repo: &str) -> String {
    let (base, _) = setup_info::base_or_placeholder();
    resolve_redirect_uri_with_base(cfg, repo, &base)
}

/// [`resolve_redirect_uri`] with the base URL injected.
///
/// Split out so the resolution rules are testable without touching
/// `RAISINDB_BASE_URL`: env vars are process-global, and several other handlers
/// in this crate (`oauth_as::helpers`, `repo_sign`, `mcp`) read that same
/// variable, so a test that set it raced their tests inside the shared test
/// binary.
fn resolve_redirect_uri_with_base(cfg: &Value, repo: &str, base: &str) -> String {
    let canonical = setup_info::oauth_redirect_uri(base, repo);
    let want_path = setup_info::oauth_callback_path(repo);

    let Some(stored) = json_str(cfg, "redirect_uri") else {
        return canonical;
    };
    match url::Url::parse(&stored) {
        Ok(mut u) => {
            if u.path().trim_end_matches('/') == want_path {
                stored
            } else {
                u.set_path(&want_path);
                u.set_query(None);
                u.set_fragment(None);
                u.to_string()
            }
        }
        Err(_) => canonical,
    }
}

/// Decrypt an integration node's `client_secret_encrypted` (base64 of the
/// standard wire format) if present. Returns `None` for public clients.
///
/// A stored-but-undecryptable secret (`RAISIN_MASTER_KEY` rotated or differing
/// between the node that stored it and the node reading it) also yields `None` —
/// indistinguishable, at the call site, from a public client. That silence is
/// how a master-key mismatch used to present as a bare `401` from the provider,
/// so it is logged loudly here. The ciphertext and plaintext are never logged.
pub(crate) fn decrypt_client_secret(node: &Node, master_key: &[u8; 32]) -> Option<String> {
    let enc = match node.properties.get("client_secret_encrypted") {
        Some(PropertyValue::String(s)) if !s.is_empty() => s,
        _ => return None,
    };
    let decoded = base64::engine::general_purpose::STANDARD.decode(enc).ok();
    let plaintext = decoded
        .as_deref()
        .and_then(|bytes| SecretBox::new(master_key).decrypt(bytes).ok());
    if plaintext.is_none() {
        tracing::error!(
            node_path = %node.path,
            "integration has a stored client_secret_encrypted that could not be decrypted; \
             the OAuth exchange will proceed WITHOUT a client_secret and the provider will \
             reject it (Microsoft: AADSTS7000218). This normally means RAISIN_MASTER_KEY \
             differs from the key the secret was stored under — re-enter the client secret \
             in the admin console, or restore the original key."
        );
    }
    plaintext
}

// The token-endpoint error redactor is shared with the background refresh grant
// in `raisin-rocksdb`; see `raisin_models::auth::oauth_error_detail`.
pub(crate) use raisin_models::auth::oauth_error_parts;

/// Read the `connected_accounts` array of a node as JSON (empty if unset).
pub(crate) fn connected_accounts(node: &Node) -> Vec<Value> {
    match node.properties.get("connected_accounts") {
        Some(pv) => match serde_json::to_value(pv) {
            Ok(Value::Array(a)) => a,
            _ => Vec::new(),
        },
        None => Vec::new(),
    }
}

/// Write a JSON array back into a node's `connected_accounts` property.
pub(crate) fn set_connected_accounts(
    node: &mut Node,
    accounts: Vec<Value>,
) -> Result<(), ApiError> {
    let pv = serde_json::from_value::<PropertyValue>(Value::Array(accounts))
        .map_err(|e| ApiError::internal(format!("failed to encode connected_accounts: {e}")))?;
    node.properties.insert("connected_accounts".to_string(), pv);
    Ok(())
}

#[cfg(test)]
mod redirect_uri_tests {
    use super::*;
    use serde_json::json;

    const BASE: &str = "https://rdb.example.test";

    /// Pure resolver — no `RAISINDB_BASE_URL` mutation, so these are safe to run
    /// in parallel with the other handlers that read that same variable.
    fn resolve(cfg: &Value, repo: &str) -> String {
        resolve_redirect_uri_with_base(cfg, repo, BASE)
    }

    #[test]
    fn missing_redirect_uri_falls_back_to_canonical() {
        assert_eq!(
            resolve(&json!({}), "studio"),
            "https://rdb.example.test/api/integrations/studio/oauth/callback"
        );
    }

    /// The exact production misconfiguration: the stored value dropped the
    /// trailing `callback`, so the provider redirected to a path that 404s.
    #[test]
    fn wrong_path_is_repaired_in_place() {
        let cfg = json!({
            "redirect_uri": "https://rdb.example.test/api/integrations/studio/oauth/"
        });
        assert_eq!(
            resolve(&cfg, "studio"),
            "https://rdb.example.test/api/integrations/studio/oauth/callback"
        );
    }

    /// A missing `{repo}` segment is also a wrong path — this is what the
    /// shipped template hint comments used to suggest.
    #[test]
    fn missing_repo_segment_is_repaired() {
        let cfg = json!({ "redirect_uri": "https://example.test/api/integrations/oauth/callback" });
        assert_eq!(
            resolve(&cfg, "studio"),
            "https://example.test/api/integrations/studio/oauth/callback"
        );
    }

    /// A correct URI on a host that differs from the configured base (proxy,
    /// custom domain) must be preserved verbatim — rewriting it to the
    /// configured base would break a working deployment.
    #[test]
    fn correct_path_on_a_different_host_is_preserved() {
        let cfg = json!({ "redirect_uri": "https://public.example/api/integrations/studio/oauth/callback" });
        assert_eq!(
            resolve(&cfg, "studio"),
            "https://public.example/api/integrations/studio/oauth/callback"
        );
    }

    #[test]
    fn unparseable_value_falls_back_to_canonical() {
        let cfg = json!({ "redirect_uri": "not a url" });
        assert_eq!(
            resolve(&cfg, "studio"),
            "https://rdb.example.test/api/integrations/studio/oauth/callback"
        );
    }

    /// Repair must drop any query/fragment so the provider sees exactly the
    /// registered redirect URI.
    #[test]
    fn repair_strips_query_and_fragment() {
        let cfg = json!({ "redirect_uri": "https://example.test/wrong?a=1#f" });
        assert_eq!(
            resolve(&cfg, "studio"),
            "https://example.test/api/integrations/studio/oauth/callback"
        );
    }
}
