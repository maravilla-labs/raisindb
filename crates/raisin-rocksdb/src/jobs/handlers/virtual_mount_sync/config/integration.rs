//! The `raisin:Integration` node: the connector and its connections.

use raisin_models::nodes::integrations::{AccountSelection, AccountSelectionError};
use raisin_models::nodes::Node;
use serde_json::Value;

/// One entry in a `raisin:Integration.connected_accounts` array — one
/// *connection*.
///
/// Defined once in `raisin-models` and re-exported here. It used to be declared
/// separately in this crate AND in `raisin-transport-http`'s connection-test
/// support module; the two drifted, which is exactly the failure mode this
/// codebase hits most often. One definition, two consumers.
pub use raisin_models::nodes::integrations::ConnectedAccount;

/// A fully-parsed `raisin:Integration` node (only the fields the engine needs).
#[derive(Debug, Clone)]
pub struct IntegrationConfig {
    pub provider_type: String,
    pub adapter_function: Option<String>,
    pub accounts: Vec<ConnectedAccount>,
    /// The integration's `api_config` object, retained verbatim (default
    /// [`Value::Null`]). Legacy provider connection settings (host/port/tls/
    /// mailbox/auth mode) that adapters read from the mount snapshot; the engine
    /// itself does not interpret it. Lowest layer of the config merge.
    pub api_config: Value,
    /// Connector-level values declared by `config_type`.
    pub config: Value,
    /// NodeType naming the per-connection config schema, if any.
    pub connection_config_type: Option<String>,
    /// Public origin this connector is reachable at, derived from the operator's
    /// stored `oauth_config.redirect_uri`.
    ///
    /// This is the ONLY per-connector public URL an operator already has to get
    /// right — the provider rejects an OAuth exchange whose redirect_uri does
    /// not match what is registered, so it is verified by the act of connecting
    /// an account. Push notification URLs derive from the same value instead of
    /// a second, separately-configured one.
    ///
    /// It replaces `RAISINDB_BASE_URL` as the primary source because that env
    /// var cannot express a multi-tenant deployment: every org is served at its
    /// own `{handle}.{base}` host, so one static value is wrong for all but one
    /// of them, and push simply could not be wired.
    pub public_origin: Option<String>,
    /// Names of per-connection config fields flagged `meta.credential`, i.e.
    /// those that belong in the adapter credential rather than the mount config
    /// (an IMAP `username`). Resolved by the caller from the config NodeType;
    /// empty when no schema is declared, which is the pre-v2 behaviour.
    pub credential_fields: Vec<String>,
}

impl IntegrationConfig {
    /// Parse a `raisin:Integration` node.
    pub fn from_node(node: &Node) -> Result<Self, String> {
        let shared = raisin_models::nodes::integrations::IntegrationConfig::from_node(node)?;
        Ok(Self {
            provider_type: shared.provider_type,
            adapter_function: shared.adapter_function,
            accounts: shared.accounts,
            api_config: shared.api_config,
            config: shared.config,
            public_origin: oauth_redirect_origin(node),
            connection_config_type: shared.connection_config_type,
            // Populated out-of-band: resolving a NodeType needs storage, and
            // this parse must stay synchronous and I/O-free (it runs on the
            // replication apply path).
            credential_fields: Vec::new(),
        })
    }

    /// Pick the connection a mount should use.
    ///
    /// Delegates to the shared truth table: an explicit `account_ref` must
    /// match; with no ref, exactly one connection resolves and anything else is
    /// an error. Notably this no longer silently falls back to `accounts[0]`
    /// when several exist — that quietly synced an arbitrary mailbox the moment
    /// a second connection was added.
    pub fn account_for(
        &self,
        account_ref: Option<&str>,
    ) -> Result<&ConnectedAccount, AccountSelectionError> {
        AccountSelection::resolve(&self.accounts, account_ref)
    }
}

/// Scheme + host (+ port) of the integration's stored `oauth_config.redirect_uri`.
///
/// Returns `None` when unset or unparseable, leaving the caller to fall back to
/// `RAISINDB_BASE_URL`. Only the ORIGIN is taken — the path is the OAuth
/// callback route and has nothing to do with the notifications route.
fn oauth_redirect_origin(node: &Node) -> Option<String> {
    let cfg = node.properties.get("oauth_config")?;
    let cfg = serde_json::to_value(cfg).ok()?;
    let raw = cfg.get("redirect_uri")?.as_str()?;
    if raw.trim().is_empty() {
        return None;
    }
    let url = url::Url::parse(raw).ok()?;
    let host = url.host_str()?;
    let scheme = url.scheme();
    Some(match url.port() {
        Some(p) => format!("{scheme}://{host}:{p}"),
        None => format!("{scheme}://{host}"),
    })
}
