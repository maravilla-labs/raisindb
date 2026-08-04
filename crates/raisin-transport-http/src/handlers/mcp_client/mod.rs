// SPDX-License-Identifier: BSL-1.1

//! Admin endpoints for OUTBOUND MCP connections.
//!
//! RaisinDB as an MCP *client*: each `raisin:McpConnection` node in
//! `raisin:system` describes one remote server whose tools become proxy
//! `raisin:Function` nodes that agents can call. Distinct from
//! [`crate::handlers::mcp`], which is the inbound direction — RaisinDB serving
//! its own tools to somebody else's client.
//!
//! Security invariants enforced throughout this module:
//! - The static credential and the OAuth tokens NEVER appear in a response body
//!   or a log line. They are stored only as AES-256-GCM ciphertext, and the only
//!   serialization of a connection is
//!   [`McpConnectionDescriptor::redacted`](raisin_mcp::client::McpConnectionDescriptor::redacted).
//! - Every endpoint is admin-gated. There is no public endpoint here yet; the
//!   OAuth callback that will need one arrives in the OAuth phase.
//! - A connection URL is validated against the operator's egress policy on
//!   every save, and again before any request is dialled.

mod connections;
mod credential;
mod probe;
mod refresh;
mod tools;

pub use connections::{
    create_connection, delete_connection, get_connection, list_connections, update_connection,
};
pub use credential::{clear_credential, set_credential};
pub use probe::test_connection;
pub use refresh::refresh_tools;
pub use tools::{list_tools, set_tool_enabled};

use raisin_crypto::SecretBox;
use raisin_mcp::client::{EgressPolicy, McpConnectionDescriptor, RemoteToolError};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use serde_json::Value;

use crate::error::ApiError;
use crate::state::{AppState, Store};

/// NodeType of a connection.
pub(crate) const CONNECTION_NODE_TYPE: &str = "raisin:McpConnection";
/// Branch holding connection configuration.
pub(crate) const CONFIG_BRANCH: &str = "main";
/// Workspace holding connection configuration, alongside `raisin:Integration`.
pub(crate) const CONFIG_WORKSPACE: &str = "raisin:system";
/// Folder holding connection nodes.
pub(crate) const CONNECTIONS_ROOT: &str = "/mcp-connections";
/// Name of that folder (a node's `parent` is a NAME, not a path).
pub(crate) const CONNECTIONS_FOLDER: &str = "mcp-connections";

/// Reject the request unless the caller is an admin.
///
/// Reuses the integrations gate rather than restating the role list: two copies
/// of "who counts as an admin" is precisely the kind of mirrored logic that
/// drifts, and here a drift would be a privilege bug.
pub(crate) use crate::handlers::integrations::require_admin;

/// Path of the connection node for `slug`.
pub(crate) fn connection_path(slug: &str) -> String {
    format!("{CONNECTIONS_ROOT}/{slug}")
}

/// Build a `raisin:system`-scoped, system-auth `NodeService` for connection
/// config reads/writes, mirroring the integrations config service.
pub(crate) fn connection_service(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    actor: &str,
) -> raisin_core::NodeService<Store> {
    let mut auth = raisin_models::auth::AuthContext::system();
    auth.user_id = Some(actor.to_string());
    state.node_service_for_context(tenant_id, repo, CONFIG_BRANCH, CONFIG_WORKSPACE, Some(auth))
}

/// Load and parse one connection, or 404.
pub(crate) async fn load_connection(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    slug: &str,
    actor: &str,
) -> Result<(Node, McpConnectionDescriptor), ApiError> {
    let path = connection_path(slug);
    let svc = connection_service(state, tenant_id, repo, actor);
    let node = svc
        .get_by_path(&path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(path.clone()))?;
    let descriptor = McpConnectionDescriptor::from_node(&node).map_err(remote_error)?;
    Ok((node, descriptor))
}

/// The operator's egress policy for this process.
pub(crate) fn egress_policy(state: &AppState) -> EgressPolicy {
    state.mcp_client_config.egress_policy()
}

/// Map a client-layer failure onto an API error.
///
/// A `Config` failure is the caller's mistake (a bad URL, a bad slug) and must
/// be a 400, not a 500 — a 500 here would send an operator hunting a server
/// fault for a typo they can fix.
pub(crate) fn remote_error(err: RemoteToolError) -> ApiError {
    match err {
        RemoteToolError::Config(_) | RemoteToolError::Protocol(_) => {
            ApiError::validation_failed(err.to_string())
        }
        RemoteToolError::AuthExpired(_) => ApiError::new(
            axum::http::StatusCode::BAD_GATEWAY,
            "MCP_AUTH_EXPIRED",
            err.to_string(),
        ),
        other => ApiError::new(
            axum::http::StatusCode::BAD_GATEWAY,
            "MCP_UNREACHABLE",
            other.to_string(),
        ),
    }
}

/// Read a node's string property, treating empty as absent.
pub(crate) fn str_prop(node: &Node, key: &str) -> Option<String> {
    match node.properties.get(key) {
        Some(PropertyValue::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

/// Write a JSON value into a node property, or remove it when `Null`.
pub(crate) fn set_prop(node: &mut Node, key: &str, value: Value) -> Result<(), ApiError> {
    if value.is_null() {
        node.properties.remove(key);
        return Ok(());
    }
    let pv = serde_json::from_value::<PropertyValue>(value)
        .map_err(|e| ApiError::internal(format!("failed to encode `{key}`: {e}")))?;
    node.properties.insert(key.to_string(), pv);
    Ok(())
}

/// Decrypt the credential a connection will present, if it has one.
///
/// Returns the bearer/API-key value for `auth_kind: static`, or the OAuth access
/// token for `auth_kind: oauth`. An undecryptable blob yields `None` and is
/// logged loudly: silently proceeding unauthenticated turns a master-key
/// mismatch into a bare 401 from the remote, which is a much harder diagnosis.
pub(crate) fn decrypt_credential(
    node: &Node,
    descriptor: &McpConnectionDescriptor,
    master_key: &[u8; 32],
) -> Option<String> {
    use raisin_mcp::client::AuthKind;

    let (property, field) = match descriptor.auth_kind {
        AuthKind::None => return None,
        AuthKind::Static => ("credential_encrypted", "value"),
        AuthKind::Oauth => ("tokens_encrypted", "access_token"),
    };
    let ciphertext = str_prop(node, property)?;
    let plaintext = SecretBox::new(master_key)
        .decrypt_json(&ciphertext)
        .ok()
        .and_then(|v| {
            v.get(field)
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|s| !s.is_empty())
        });

    if plaintext.is_none() {
        tracing::error!(
            node_path = %node.path,
            property,
            "mcp connection has a stored credential that could not be decrypted; the request \
             will proceed UNAUTHENTICATED and the remote will reject it. This normally means \
             RAISIN_MASTER_KEY differs from the key the credential was stored under — re-enter \
             the credential, or restore the original key."
        );
    }
    plaintext
}
