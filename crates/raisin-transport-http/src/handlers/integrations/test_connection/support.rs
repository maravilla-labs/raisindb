// SPDX-License-Identifier: BSL-1.1

//! Pure helpers for the connection test: credential building (refresh_token
//! stripped), capability parsing, error-code mapping, and diagnostic scrubbing.
//! Kept free of I/O so they are directly unit-testable.

use raisin_models::nodes::Node;
use serde_json::{json, Value};

use super::{Capabilities, Probe, PROBE_LIMIT};

impl Capabilities {
    /// Conservative fallback when the adapter has no usable `capabilities`
    /// operation: assume read-only and nothing else.
    pub(super) fn fallback() -> Self {
        Self {
            can_read: true,
            ..Self::default()
        }
    }

    /// Parse an adapter's `capabilities` return value, falling back when it is
    /// null/absent or not a decodable capabilities object.
    pub(super) fn from_value(value: &Value) -> Self {
        if value.is_null() {
            return Self::fallback();
        }
        serde_json::from_value(value.clone()).unwrap_or_else(|_| Self::fallback())
    }
}

/// Build a [`Probe`] from a `list` result: item **names** only, capped. URLs
/// and other fields (which may embed tokens) are never surfaced.
pub(super) fn probe_from_list(value: &Value) -> Probe {
    let items = value
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let sample = items
        .iter()
        .filter_map(|it| it.get("name").and_then(|n| n.as_str()).map(String::from))
        .take(PROBE_LIMIT)
        .collect();
    Probe {
        items_seen: items.len(),
        sample,
    }
}

/// Build the adapter `credential` object. **The `refresh_token` is never
/// included.** `username` (the connected account's verified email) is included
/// when present so token-based mechanisms like IMAP XOAUTH2 can auto-fill it —
/// mirrors the sync engine's `build_credential`.
pub(super) fn build_credential(
    provider_type: &str,
    account_id: &str,
    username: Option<&str>,
    tokens: &Value,
) -> Value {
    let mut cred = json!({
        "access_token": tokens.get("access_token").and_then(|v| v.as_str()).unwrap_or(""),
        "account_id": account_id,
        "provider_type": provider_type,
    });
    if let Some(u) = username {
        cred["username"] = Value::String(u.to_string());
    }
    cred
}

/// Map an adapter error message to a stable diagnostic `code`. Mirrors the sync
/// engine's `AdapterError::classify` reserved codes.
pub(in crate::handlers::integrations) fn adapter_error_code(msg: &str) -> &'static str {
    let m = msg.to_ascii_lowercase();
    if m.contains("auth_expired") {
        "auth_expired"
    } else if m.contains("rate_limited") {
        "rate_limited"
    } else if m.contains("conflict") {
        "conflict"
    } else if m.contains("invocation_failed") {
        "invocation_failed"
    } else {
        "adapter_error"
    }
}

/// Whether an adapter error signals expired/rejected auth.
pub(super) fn error_is_auth_expired(msg: &str) -> bool {
    adapter_error_code(msg) == "auth_expired"
}

/// Defensive scrub: never let a token-shaped substring leak into a diagnostic
/// message. Tokens should never appear in an adapter error, but a misbehaving
/// adapter could echo one — redact anything suspicious.
pub(in crate::handlers::integrations) fn sanitize(msg: &str) -> String {
    let mut out = String::new();
    for word in msg.split_whitespace() {
        let lower = word.to_ascii_lowercase();
        if lower.starts_with("ya29.")
            || lower.starts_with("bearer")
            || lower.starts_with("eyj") // JWT
            || word.len() > 64
        {
            out.push_str("[redacted] ");
        } else {
            out.push_str(word);
            out.push(' ');
        }
    }
    out.trim().to_string()
}

/// Read a non-empty string property.
#[cfg(feature = "storage-rocksdb")]
pub(super) fn string_prop(node: &Node, key: &str) -> Option<String> {
    use raisin_models::nodes::properties::PropertyValue;
    match node.properties.get(key)? {
        PropertyValue::String(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

/// Decrypt an account's tokens and build the §4.1 `credential` — **without the
/// refresh_token**. Returns `None` when the account is absent or has no
/// decryptable access token.
#[cfg(feature = "storage-rocksdb")]
pub(in crate::handlers::integrations) fn resolve_credential(
    state: &crate::state::AppState,
    node: &Node,
    provider_type: &str,
    account_id: &str,
) -> Option<Value> {
    use raisin_crypto::SecretBox;
    let key = state.get_master_key().ok()?;
    let accounts = crate::handlers::integrations::connected_accounts(node);
    let account = accounts
        .iter()
        .find(|a| a.get("id").and_then(|v| v.as_str()) == Some(account_id))?;
    let enc = account.get("tokens_encrypted").and_then(|v| v.as_str())?;
    let tokens = SecretBox::new(&key).decrypt_json(enc).ok()?;
    // Require a decryptable access token; otherwise the account is unusable.
    tokens.get("access_token").and_then(|v| v.as_str())?;
    let username = account.get("subject").and_then(|v| v.as_str());
    Some(build_credential(
        provider_type,
        account_id,
        username,
        &tokens,
    ))
}

/// Read-only `mount` snapshot for a one-off adapter invocation (the connection
/// test, the browse endpoint).
///
/// **Must mirror the sync engine's `build_mount_snapshot`**
/// (`raisin-rocksdb/.../virtual_mount_sync/adapter.rs`), including the resolved
/// `config` key. It did not, and the gap was silent: `config` is the layer that
/// carries connector- and CONNECTION-level settings, so a probe built without it
/// saw only `api_config` and the caller's `sync_config`. An ms-graph connection
/// holding `tenant_id`, or a `principal` naming a shared mailbox, was invisible
/// to the probe — Test connection would read `/me` and report success while the
/// real sync read a different mailbox entirely. Anything that says "this is what
/// a sync would do" has to be built from the same layers a sync uses.
///
/// `caller_sync` carries the mount's own `sync_config` keys — the ms-graph
/// `resource`, a calendar `window` — layered *under* the probe-owned keys so a
/// caller can select the surface to test but never widen the probe
/// (`max_items_per_sync` stays capped, `ephemeral` stays off).
#[cfg(feature = "storage-rocksdb")]
pub(in crate::handlers::integrations) fn mount_snapshot(
    remote_root: &Option<String>,
    node: &Node,
    account_id: Option<&str>,
    caller_sync: Option<&Value>,
) -> Value {
    let mut sync = serde_json::Map::new();
    if let Some(Value::Object(given)) = caller_sync {
        for (k, v) in given {
            sync.insert(k.clone(), v.clone());
        }
    }
    // Probe-owned keys always win.
    sync.insert("mode".into(), json!("poll"));
    sync.insert("interval_seconds".into(), json!(300));
    sync.insert("include_patterns".into(), json!([]));
    sync.insert("exclude_patterns".into(), json!([]));
    sync.insert("ephemeral".into(), json!(false));
    sync.insert("ttl_seconds".into(), Value::Null);
    sync.insert("max_items_per_sync".into(), json!(PROBE_LIMIT));
    let sync_config = Value::Object(sync);

    let api_config = crate::handlers::integrations::json_prop(node, "api_config");
    let connector_config = crate::handlers::integrations::json_prop(node, "config");
    let account_config = account_id
        .map(|id| account_config(node, id))
        .unwrap_or(Value::Null);

    // Same precedence, same function as the engine: api_config < connector <
    // connection < sync_config.
    let merged = raisin_models::nodes::integrations::merge_config(
        &api_config,
        &connector_config,
        &account_config,
        &sync_config,
    );

    json!({
        "mount_id": "connection-test",
        "remote_root": remote_root,
        "mount_path": "/",
        "api_config": api_config,
        "sync_config": sync_config,
        "config": merged,
    })
}

/// The per-connection `config` object of one connected account (or `Null`).
///
/// This is the layer that distinguishes two connections on the same connector —
/// two Entra tenants, two shared mailboxes — so a probe that omits it tests
/// something other than what the operator selected.
#[cfg(feature = "storage-rocksdb")]
pub(super) fn account_config(node: &Node, account_id: &str) -> Value {
    crate::handlers::integrations::connected_accounts(node)
        .into_iter()
        .find(|a| a.get("id").and_then(|v| v.as_str()) == Some(account_id))
        .and_then(|a| a.get("config").cloned())
        .unwrap_or(Value::Null)
}

/// Persist `capabilities` + `capabilities_checked_at` onto the integration node.
#[cfg(feature = "storage-rocksdb")]
pub(super) async fn cache_capabilities(
    svc: &raisin_core::NodeService<crate::state::Store>,
    node: &mut Node,
    caps: &Capabilities,
) -> Result<(), crate::error::ApiError> {
    use crate::error::ApiError;
    use raisin_models::nodes::properties::PropertyValue;
    let value = serde_json::to_value(caps)
        .map_err(|e| ApiError::internal(format!("failed to encode capabilities: {e}")))?;
    let pv = serde_json::from_value::<PropertyValue>(value)
        .map_err(|e| ApiError::internal(format!("failed to encode capabilities: {e}")))?;
    node.properties.insert("capabilities".to_string(), pv);
    node.properties.insert(
        "capabilities_checked_at".to_string(),
        PropertyValue::String(chrono::Utc::now().to_rfc3339()),
    );
    svc.update_node(node.clone()).await?;
    Ok(())
}

/// System auth context for privileged config/function reads.
#[cfg(feature = "storage-rocksdb")]
pub(super) fn system_auth() -> raisin_models::auth::AuthContext {
    let mut auth = raisin_models::auth::AuthContext::system();
    auth.user_id = Some("integration-test".to_string());
    auth
}

#[cfg(test)]
mod tests {
    use super::super::Capabilities;
    use super::*;

    #[test]
    fn credential_never_contains_refresh_token() {
        let tokens = json!({ "access_token": "at-123", "refresh_token": "rt-secret" });
        let cred = build_credential("google-drive", "acct-1", None, &tokens);
        assert_eq!(cred.get("access_token").unwrap(), "at-123");
        assert!(cred.get("refresh_token").is_none());
        assert_eq!(cred.get("account_id").unwrap(), "acct-1");
        assert_eq!(cred.get("provider_type").unwrap(), "google-drive");
        // No username when the account has no subject.
        assert!(cred.get("username").is_none());
        // The whole serialized object must not mention the refresh token.
        assert!(!cred.to_string().contains("rt-secret"));
    }

    #[test]
    fn credential_carries_username_when_present() {
        let tokens = json!({ "access_token": "at-123", "refresh_token": "rt-secret" });
        let cred = build_credential("gmail", "acct-1", Some("alice@example.com"), &tokens);
        assert_eq!(cred.get("username").unwrap(), "alice@example.com");
        assert!(!cred.to_string().contains("rt-secret"));
    }

    /// An integration node carrying connector-level `config`, an `api_config`
    /// and one connection whose own `config` names a shared mailbox.
    #[cfg(feature = "storage-rocksdb")]
    fn integration_node() -> Node {
        serde_json::from_value(json!({
            "name": "ms-graph",
            "node_type": "raisin:Integration",
            "properties": {
                "api_config": { "legacy": "lowest", "resource": "mail" },
                "config": { "default_tenant_id": "tenant-guid", "legacy": "connector" },
                "connected_accounts": [{
                    "id": "acct-1",
                    "config": { "principal": "sales@contoso.com", "legacy": "connection" }
                }]
            }
        }))
        .expect("node")
    }

    /// The probe must see the same merged `config` a real sync would, or it
    /// tests a different mailbox than the one the mount syncs.
    #[cfg(feature = "storage-rocksdb")]
    #[test]
    fn mount_snapshot_merges_connector_and_connection_config() {
        let node = integration_node();
        let snap = mount_snapshot(
            &Some("inbox".into()),
            &node,
            Some("acct-1"),
            Some(&json!({ "resource": "calendar" })),
        );
        let cfg = snap.get("config").expect("merged config is present");

        // Connection layer reaches the adapter — this is the shared mailbox.
        assert_eq!(cfg.get("principal").unwrap(), "sales@contoso.com");
        // Connector layer too.
        assert_eq!(cfg.get("default_tenant_id").unwrap(), "tenant-guid");
        // Caller's sync_config wins over the api_config default.
        assert_eq!(cfg.get("resource").unwrap(), "calendar");
        // Precedence across all four layers, highest wins.
        assert_eq!(cfg.get("legacy").unwrap(), "connection");
        // Probe-owned caps survive the merge and are not widened by the caller.
        assert_eq!(cfg.get("max_items_per_sync").unwrap(), PROBE_LIMIT);
        assert_eq!(cfg.get("ephemeral").unwrap(), false);
    }

    /// With no account selected there is no connection layer, but connector and
    /// api config must still reach the adapter.
    #[cfg(feature = "storage-rocksdb")]
    #[test]
    fn mount_snapshot_without_account_still_merges_connector_config() {
        let snap = mount_snapshot(&None, &integration_node(), None, None);
        let cfg = snap.get("config").expect("merged config is present");
        assert_eq!(cfg.get("default_tenant_id").unwrap(), "tenant-guid");
        assert!(cfg.get("principal").is_none());
        assert_eq!(cfg.get("legacy").unwrap(), "connector");
    }

    #[test]
    fn error_code_mapping() {
        assert_eq!(adapter_error_code("Error: auth_expired"), "auth_expired");
        assert_eq!(
            adapter_error_code("rate_limited by provider"),
            "rate_limited"
        );
        assert_eq!(adapter_error_code("etag conflict"), "conflict");
        assert_eq!(
            adapter_error_code("invocation_failed: boom"),
            "invocation_failed"
        );
        assert_eq!(adapter_error_code("something else"), "adapter_error");
    }

    #[test]
    fn auth_expired_maps_to_expired_status() {
        assert!(error_is_auth_expired("Error: auth_expired"));
        assert!(!error_is_auth_expired("rate_limited"));
    }

    #[test]
    fn probe_extracts_names_only_capped() {
        let value = json!({
            "items": [
                { "name": "a.txt", "web_url": "https://x/tok?access_token=SECRET" },
                { "name": "b.txt" },
                { "name": "c.txt" },
            ]
        });
        let probe = probe_from_list(&value);
        assert_eq!(probe.items_seen, 3);
        assert_eq!(probe.sample, vec!["a.txt", "b.txt", "c.txt"]);
        // No URL/token content leaks into the sample.
        assert!(!probe.sample.iter().any(|s| s.contains("access_token")));
    }

    #[test]
    fn capabilities_fallback_is_read_only() {
        let caps = Capabilities::from_value(&Value::Null);
        assert!(caps.can_read);
        assert!(!caps.can_write);
        assert!(!caps.supports_changes);
    }

    /// An integration node with no config of its own.
    #[cfg(feature = "storage-rocksdb")]
    fn bare_node() -> Node {
        serde_json::from_value(json!({ "name": "c", "node_type": "raisin:Integration" }))
            .expect("node")
    }

    #[cfg(feature = "storage-rocksdb")]
    #[test]
    fn mount_snapshot_forwards_resource_but_caps_the_probe() {
        let caller = json!({ "resource": "calendar", "max_items_per_sync": 100_000 });
        let snap = mount_snapshot(&Some("cal-1".into()), &bare_node(), None, Some(&caller));
        let sync = &snap["sync_config"];
        // The surface selector reaches the adapter...
        assert_eq!(sync["resource"], "calendar");
        // ...but probe-owned keys are not overridable.
        assert_eq!(sync["max_items_per_sync"], json!(PROBE_LIMIT));
        assert_eq!(sync["ephemeral"], json!(false));
        assert_eq!(snap["remote_root"], "cal-1");
    }

    #[cfg(feature = "storage-rocksdb")]
    #[test]
    fn mount_snapshot_without_caller_sync_keeps_defaults() {
        let snap = mount_snapshot(&None, &bare_node(), None, None);
        assert_eq!(snap["sync_config"]["mode"], "poll");
        assert!(snap["sync_config"].get("resource").is_none());
    }

    #[test]
    fn sanitize_redacts_tokens() {
        let s = sanitize("failed with ya29.abcdef and bearer xyz");
        assert!(!s.contains("ya29"));
        assert!(s.contains("[redacted]"));
    }
}
