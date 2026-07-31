//! Typed configuration for the virtual-mount sync engine.
//!
//! Loads and validates `raisin:VirtualMount` and `raisin:Integration` nodes into
//! strongly-typed structs, defines the normalized external-item / change shapes
//! adapters return, and provides the built-in Rust default mapping and the
//! include/exclude glob filter.

use raisin_models::nodes::integrations::{AccountSelection, AccountSelectionError};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// The workspace that holds integration/mount configuration in every repo.
pub const SYSTEM_WORKSPACE: &str = "raisin:system";

/// Actor stamped on every sync-materialized write. Downstream writeback
/// triggers filter on this to avoid feedback loops.
pub const SYNC_ACTOR: &str = "virtual-mount-sync";

/// Default number of consecutive failures before a mount is marked degraded.
pub const DEGRADE_THRESHOLD: u64 = 5;

/// Default per-sync item cap when a mount does not specify one.
pub const DEFAULT_MAX_ITEMS: u64 = 500;

/// Default poll interval (seconds) when a mount omits `interval_seconds`.
pub const DEFAULT_INTERVAL_SECS: u64 = 300;

/// Parsed `sync_config` object of a `raisin:VirtualMount`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    /// `"poll" | "webhook" | "hybrid"`. `webhook` mounts are skipped by the
    /// periodic driver (they are driven by inbound webhooks instead).
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default = "default_interval")]
    pub interval_seconds: u64,
    #[serde(default)]
    pub include_patterns: Vec<String>,
    #[serde(default)]
    pub exclude_patterns: Vec<String>,
    #[serde(default)]
    pub ephemeral: bool,
    #[serde(default)]
    pub ttl_seconds: Option<u64>,
    #[serde(default = "default_max_items")]
    pub max_items_per_sync: u64,
}

fn default_mode() -> String {
    "poll".to_string()
}
fn default_interval() -> u64 {
    DEFAULT_INTERVAL_SECS
}
fn default_max_items() -> u64 {
    DEFAULT_MAX_ITEMS
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            mode: default_mode(),
            interval_seconds: default_interval(),
            include_patterns: Vec::new(),
            exclude_patterns: Vec::new(),
            ephemeral: false,
            ttl_seconds: None,
            max_items_per_sync: default_max_items(),
        }
    }
}

/// Engine-managed `state` object of a `raisin:VirtualMount`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MountState {
    #[serde(default)]
    pub last_sync_token: Option<String>,
    /// Unix epoch seconds of the last successful sync write.
    #[serde(default)]
    pub last_sync_at: Option<i64>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub consecutive_failures: u64,
    /// `"ok" | "syncing" | "auth_required" | "degraded" | "misconfigured"`.
    #[serde(default)]
    pub status: Option<String>,
    /// Fencing token of the last sync that wrote this state (cluster safety).
    #[serde(default)]
    pub last_fencing_token: Option<u64>,
    /// `false` when the mount requests `write_through` writeback but the engine
    /// or adapter cannot honour it (v1 has no writeback implementation). `None`
    /// when writeback was never requested. The UI reads this to explain the
    /// limitation; it never makes the sync fatal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub writeback_supported: Option<bool>,

    // ---- push / webhook subscription (all engine-managed, optional) ----
    /// Stable, unguessable per-mount token that gates the public notifications
    /// endpoint. Format: `"{mount_id}.{nanoid}"`. Generated once by the engine
    /// (see [`super::subscription::ensure_mount_token`]) and embedded in the
    /// `notification_url` handed to the provider on `subscribe`. The HTTP
    /// notifications endpoint parses `mount_id` from the prefix, loads this
    /// mount, and constant-time compares the incoming token against this value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_mount_token: Option<String>,
    /// Provider-side subscription/channel id returned by the adapter's
    /// `subscribe` op. Its presence (with a future `push_expires_at`) marks the
    /// mount as having a live push subscription.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_subscription_id: Option<String>,
    /// Optional shared secret returned by `subscribe` (e.g. Graph `clientState`,
    /// Google channel token). The notifications endpoint MAY verify it when the
    /// provider echoes it; the `push_mount_token` in the URL is the primary auth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_secret: Option<String>,
    /// ISO-8601 expiry of the provider subscription. The renewal job refreshes
    /// subscriptions whose expiry is within one day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_expires_at: Option<String>,
    /// The `notification_url` last handed to the provider (for renew/debug).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_notification_url: Option<String>,
    /// `"active" | "failed" | "unsupported"` (or `None` when never attempted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_status: Option<String>,
    /// Last push-subscription error message, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_last_error: Option<String>,
}

impl MountState {
    /// Whether this mount currently has a usable push subscription: an id is
    /// present, status is `active`, and the expiry (if known) is in the future.
    /// `now` is Unix epoch seconds.
    pub fn has_active_push(&self, now: i64) -> bool {
        if self.push_status.as_deref() != Some("active") {
            return false;
        }
        if self.push_subscription_id.is_none() {
            return false;
        }
        match &self.push_expires_at {
            None => true,
            Some(iso) => parse_iso_epoch(iso).map(|t| t > now).unwrap_or(true),
        }
    }

    /// Whether the push subscription expires within `window_secs` of `now`
    /// (Unix seconds) and should be renewed. Only meaningful for active subs.
    pub fn push_expires_within(&self, now: i64, window_secs: i64) -> bool {
        match self.push_expires_at.as_deref().and_then(parse_iso_epoch) {
            Some(exp) => exp <= now + window_secs,
            None => false,
        }
    }
}

/// Parse an ISO-8601 / RFC-3339 timestamp into Unix epoch seconds.
pub fn parse_iso_epoch(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp())
}

/// Parsed `write_config` object of a `raisin:VirtualMount`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteConfig {
    /// `"off" | "write_through"`. v1 only honours `off`.
    #[serde(default = "default_writeback")]
    pub writeback: String,
    /// `"remote_wins" | "error"`.
    #[serde(default)]
    pub conflict: Option<String>,
}

fn default_writeback() -> String {
    "off".to_string()
}

impl Default for WriteConfig {
    fn default() -> Self {
        Self {
            writeback: default_writeback(),
            conflict: None,
        }
    }
}

impl WriteConfig {
    /// Whether this mount asks for write-through writeback.
    pub fn wants_write_through(&self) -> bool {
        self.writeback.eq_ignore_ascii_case("write_through")
    }
}

/// A fully-parsed `raisin:VirtualMount` node.
#[derive(Debug, Clone)]
pub struct MountConfig {
    pub mount_id: String,
    pub integration_ref: String,
    pub account_ref: Option<String>,
    pub target_workspace: String,
    /// Branch the mount materializes into (the mount config itself always lives
    /// on the repo's config branch). Defaults to `"main"` when unset.
    pub target_branch: String,
    pub mount_path: String,
    pub remote_root: Option<String>,
    pub adapter_function: Option<String>,
    pub mapping_function: Option<String>,
    pub enabled: bool,
    pub sync_config: SyncConfig,
    /// The original `sync_config` object as authored on the node, retained
    /// verbatim. The typed [`SyncConfig`] above only captures the fields the
    /// engine itself acts on; adapters need the full object (host/port/tls/
    /// mailbox/username/auth and any provider-specific keys), so the raw value
    /// is what is forwarded to them in the mount snapshot.
    pub sync_config_raw: Value,
    pub write_config: WriteConfig,
    pub state: MountState,
}

impl MountConfig {
    /// Parse a `raisin:VirtualMount` node. Returns an error only when a
    /// required field is missing or malformed.
    pub fn from_node(node: &Node) -> Result<Self, String> {
        let target_workspace = req_string(node, "target_workspace")?;
        let mount_path = req_string(node, "mount_path")?;
        let integration_ref = req_string(node, "integration_ref")?;
        Ok(Self {
            mount_id: node.id.clone(),
            integration_ref,
            account_ref: opt_string(node, "account_ref"),
            target_workspace,
            target_branch: opt_string(node, "target_branch")
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "main".to_string()),
            mount_path: normalize_path(&mount_path),
            remote_root: opt_string(node, "remote_root"),
            adapter_function: opt_string(node, "adapter_function"),
            mapping_function: opt_string(node, "mapping_function"),
            enabled: opt_bool(node, "enabled").unwrap_or(true),
            sync_config: parse_object(node, "sync_config").unwrap_or_default(),
            sync_config_raw: raw_value(node, "sync_config"),
            write_config: parse_object(node, "write_config").unwrap_or_default(),
            state: parse_object(node, "state").unwrap_or_default(),
        })
    }

    /// Effective poll interval honoring failure backoff:
    /// `interval * 2^min(consecutive_failures, 5)`.
    pub fn effective_interval_secs(&self) -> u64 {
        let base = self.sync_config.interval_seconds.max(1);
        let shift = self.state.consecutive_failures.min(DEGRADE_THRESHOLD);
        base.saturating_mul(1u64 << shift)
    }
}

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

/// A normalized external object as returned by an adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalItem {
    pub external_id: String,
    pub name: String,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub size_bytes: Option<i64>,
    #[serde(default)]
    pub is_folder: bool,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub modified_at: Option<String>,
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub web_url: Option<String>,
    #[serde(default)]
    pub download_url: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

/// One entry in a `get_changes` feed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Change {
    #[serde(rename = "type")]
    pub kind: String,
    pub item: ExternalItem,
    #[serde(default)]
    pub relative_path: String,
}

/// A `get_changes` page.
#[derive(Debug, Clone, Deserialize)]
pub struct ChangesPage {
    #[serde(default)]
    pub items: Vec<Change>,
    #[serde(default)]
    pub next_token: Option<String>,
}

/// A `list` page.
#[derive(Debug, Clone, Deserialize)]
pub struct ListPage {
    #[serde(default)]
    pub items: Vec<ExternalItem>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

/// The node shape a mapping produces (built-in default or a mapping function).
#[derive(Debug, Clone)]
pub struct MappedNode {
    pub node_type: String,
    pub name: Option<String>,
    pub properties: serde_json::Map<String, Value>,
}

/// Built-in Rust default mapping (§4.2): folder → `raisin:Folder`, everything
/// else → `raisin:Node` with title + a `meta` object carrying mime/size/urls and
/// provider passthrough. Zero function invocations.
///
/// Divergence from the frozen contract, which names `raisin:Asset`: that type
/// requires a binary `file` Resource, which a link-only v1 virtual node does not
/// have (see risk #7 "links only in v1"). `raisin:Node` is the correct permissive
/// carrier for metadata-only mounts; a mount that wants `raisin:Asset` (with real
/// content sync) supplies a custom `mapping_function`.
pub fn default_mapping(item: &ExternalItem) -> MappedNode {
    let mut props = serde_json::Map::new();
    if item.is_folder {
        MappedNode {
            node_type: "raisin:Folder".to_string(),
            name: Some(item.name.clone()),
            properties: props,
        }
    } else {
        props.insert("title".to_string(), Value::String(item.name.clone()));
        let mut meta = serde_json::Map::new();
        if let Some(mt) = &item.mime_type {
            meta.insert("mime_type".to_string(), Value::String(mt.clone()));
        }
        if let Some(sz) = item.size_bytes {
            meta.insert("size".to_string(), Value::from(sz));
        }
        if let Some(u) = &item.web_url {
            meta.insert("web_url".to_string(), Value::String(u.clone()));
        }
        if let Some(u) = &item.download_url {
            meta.insert("download_url".to_string(), Value::String(u.clone()));
        }
        if let Some(Value::Object(m)) = &item.metadata {
            for (k, v) in m {
                meta.insert(k.clone(), v.clone());
            }
        }
        if !meta.is_empty() {
            props.insert("meta".to_string(), Value::Object(meta));
        }
        MappedNode {
            node_type: "raisin:Node".to_string(),
            name: Some(item.name.clone()),
            properties: props,
        }
    }
}

/// Apply include/exclude globs to a relative path. Exclude wins over include.
/// An empty `include` list includes everything; a non-empty one requires a
/// match.
pub fn passes_filters(rel_path: &str, include: &[String], exclude: &[String]) -> bool {
    for pat in exclude {
        if glob_matches(pat, rel_path) {
            return false;
        }
    }
    if include.is_empty() {
        return true;
    }
    include.iter().any(|p| glob_matches(p, rel_path))
}

fn glob_matches(pattern: &str, path: &str) -> bool {
    match glob::Pattern::new(pattern) {
        Ok(p) => p.matches(path),
        // A malformed pattern never matches (fail closed for include, open for
        // exclude is handled by the caller's ordering).
        Err(_) => false,
    }
}

// ---- property helpers ----

fn normalize_path(p: &str) -> String {
    let trimmed = p.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    }
}

fn req_string(node: &Node, key: &str) -> Result<String, String> {
    opt_string(node, key).ok_or_else(|| format!("missing required property `{key}`"))
}

fn opt_string(node: &Node, key: &str) -> Option<String> {
    match node.properties.get(key)? {
        PropertyValue::String(s) => Some(s.clone()),
        _ => None,
    }
}

fn opt_bool(node: &Node, key: &str) -> Option<bool> {
    match node.properties.get(key)? {
        PropertyValue::Boolean(b) => Some(*b),
        _ => None,
    }
}

fn parse_object<T: for<'de> Deserialize<'de>>(node: &Node, key: &str) -> Option<T> {
    let pv = node.properties.get(key)?;
    let json = serde_json::to_value(pv).ok()?;
    serde_json::from_value(json).ok()
}

/// The raw JSON value of a property, or [`Value::Null`] when absent or
/// unserializable. Used to forward author-supplied config objects verbatim.
fn raw_value(node: &Node, key: &str) -> Value {
    node.properties
        .get(key)
        .and_then(|pv| serde_json::to_value(pv).ok())
        .unwrap_or(Value::Null)
}

/// Convert a mapped property map plus reserved virtual metadata into node
/// properties (`PropertyValue` map). Reserved keys always overwrite whatever a
/// mapping produced.
pub fn build_properties(
    mapped: &serde_json::Map<String, Value>,
    virt: &super::materializer::VirtualMeta,
) -> HashMap<String, PropertyValue> {
    let mut out: HashMap<String, PropertyValue> = HashMap::new();
    for (k, v) in mapped {
        if k.starts_with("__") {
            continue; // engine owns reserved props
        }
        if let Ok(pv) = serde_json::from_value::<PropertyValue>(v.clone()) {
            out.insert(k.clone(), pv);
        }
    }
    out.insert("__virtual".to_string(), PropertyValue::Boolean(true));
    out.insert(
        "__mount_id".to_string(),
        PropertyValue::String(virt.mount_id.clone()),
    );
    out.insert(
        "__external_id".to_string(),
        PropertyValue::String(virt.external_id.clone()),
    );
    if let Some(etag) = &virt.etag {
        out.insert("__etag".to_string(), PropertyValue::String(etag.clone()));
    }
    out.insert(
        "__synced_at".to_string(),
        PropertyValue::String(virt.synced_at.clone()),
    );
    out
}
