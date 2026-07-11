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

mod client_secret;
mod manage;
mod notifications;
mod oauth_callback;
mod oauth_start;
mod setup_info;
pub mod state_store;
mod test_connection;

pub use client_secret::set_client_secret;
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

/// The `oauth_config` object of an integration node as raw JSON (or `Null`).
pub(crate) fn oauth_config(node: &Node) -> Value {
    node.properties
        .get("oauth_config")
        .and_then(|pv| serde_json::to_value(pv).ok())
        .unwrap_or(Value::Null)
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

/// Decrypt an integration node's `client_secret_encrypted` (base64 of the
/// standard wire format) if present. Returns `None` for public clients.
pub(crate) fn decrypt_client_secret(node: &Node, master_key: &[u8; 32]) -> Option<String> {
    let enc = match node.properties.get("client_secret_encrypted")? {
        PropertyValue::String(s) if !s.is_empty() => s,
        _ => return None,
    };
    let bytes = base64::engine::general_purpose::STANDARD.decode(enc).ok()?;
    SecretBox::new(master_key).decrypt(&bytes).ok()
}

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
