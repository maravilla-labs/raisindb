// SPDX-License-Identifier: BSL-1.1

//! Pure helpers for the connection test: credential building (refresh_token
//! stripped), capability parsing, error-code mapping, and diagnostic scrubbing.
//! Kept free of I/O so they are directly unit-testable.

use raisin_models::nodes::Node;
use serde_json::{json, Value};

use super::{Capabilities, Probe, PROBE_LIMIT};

/// Parse an adapter's `capabilities` return value, falling back to read-only
/// when it is null/absent or not a decodable capabilities object.
///
/// A free function rather than an inherent method because [`Capabilities`] is
/// now owned by `raisin-models` — the whole point of the move. The parse and
/// the fallback are the engine's own, so the panel can no longer disagree with
/// the sync about what an adapter answered.
pub(super) fn capabilities_from_value(value: &Value) -> Capabilities {
    Capabilities::from_adapter_value(value).unwrap_or_else(Capabilities::fallback)
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

/// Decrypt a connection into the adapter `credential`.
///
/// Both connection shapes reach this: OAuth (a `tokens_encrypted` blob) and
/// credential-based (a `secrets_encrypted` map — an API key, an IMAP app
/// password). The previous implementation read only `tokens_encrypted` and
/// required an `access_token` in it, so **every** connection made through
/// `Add connection` failed the probe with `missing_credential` no matter what
/// the operator typed. The sync engine had the same bug and was fixed; this
/// copy was not, which is how a connector could sync while Test connection
/// insisted the credential was missing.
///
/// Assembly is delegated to the engine's own `build_credential`, so the probe
/// and a real sync build the credential from the same code — including the
/// structural guarantee that no `refresh_token` reaches an adapter.
///
/// Returns `None` when the account is absent, when nothing decrypts, or when
/// what decrypted cannot authenticate anything: no `access_token` and no
/// secrets. Credential-flagged *config* alone (a username with no password) is
/// not a usable credential.
#[cfg(feature = "storage-rocksdb")]
pub(in crate::handlers::integrations) async fn resolve_credential(
    state: &crate::state::AppState,
    tenant_id: &str,
    repo: &str,
    node: &Node,
    provider_type: &str,
    account_id: &str,
) -> Option<Value> {
    use raisin_crypto::SecretBox;
    use raisin_models::nodes::integrations::{build_credential, ConnectedAccount};

    let key = state.get_master_key().ok()?;
    let secret_box = SecretBox::new(&key);

    let account: ConnectedAccount = crate::handlers::integrations::connected_accounts(node)
        .into_iter()
        .filter_map(|v| serde_json::from_value::<ConnectedAccount>(v).ok())
        .find(|a| a.id == account_id)?;

    // Either half may be absent — that is the whole point. A connection with
    // neither is unusable.
    let tokens = account
        .tokens_encrypted
        .as_deref()
        .and_then(|enc| secret_box.decrypt_json(enc).ok());
    let secrets = account
        .secrets_encrypted
        .as_deref()
        .and_then(|enc| secret_box.decrypt_json(enc).ok());

    let has_token = tokens
        .as_ref()
        .and_then(|t| t.get("access_token"))
        .and_then(|v| v.as_str())
        .is_some_and(|t| !t.is_empty());
    let has_secret = matches!(secrets.as_ref(), Some(Value::Object(m)) if !m.is_empty());
    if !has_token && !has_secret {
        return None;
    }

    let fields = credential_fields(state, tenant_id, repo, node).await;
    Some(build_credential(
        provider_type,
        &account,
        tokens.as_ref(),
        secrets.as_ref(),
        &fields,
    ))
}

/// The connection-config fields the connector marks `meta.credential: true`.
///
/// These are the non-secret halves of a credential — an IMAP username beside
/// its encrypted password. Resolved through [`NodeTypeResolver`] so `extends`
/// and mixins are honoured, exactly as the sync engine resolves them: a probe
/// built from a different field set is a probe that can report success on a
/// login the sync would never make.
///
/// Empty when the connector declares no `connection_config_type`, or when that
/// type cannot be resolved — a misconfiguration worth a log line, not a reason
/// to refuse a credential that may not need those fields at all.
#[cfg(feature = "storage-rocksdb")]
async fn credential_fields(
    state: &crate::state::AppState,
    tenant_id: &str,
    repo: &str,
    node: &Node,
) -> Vec<String> {
    use raisin_core::services::node_type_resolver::NodeTypeResolver;
    use raisin_models::nodes::properties::PropertyValue;

    let Some(type_name) = string_prop(node, "connection_config_type") else {
        return Vec::new();
    };
    let branch = crate::handlers::integrations::config_branch(state, tenant_id, repo).await;
    let resolver = NodeTypeResolver::new(
        state.storage().clone(),
        tenant_id.to_string(),
        repo.to_string(),
        branch,
    );
    match resolver.resolve(&type_name).await {
        Ok(resolved) => resolved
            .resolved_properties
            .iter()
            .filter(|p| {
                p.meta
                    .as_ref()
                    .and_then(|m| m.get("credential"))
                    .is_some_and(|v| matches!(v, PropertyValue::Boolean(true)))
            })
            .filter_map(|p| p.name.clone())
            .collect(),
        Err(e) => {
            tracing::warn!(
                config_type = %type_name,
                error = %e,
                "could not resolve connection config type; credential fields unavailable"
            );
            Vec::new()
        }
    }
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
///
/// Goes through the connector's accounts lease, which re-reads the node inside
/// it, because the node this probe started from was loaded BEFORE a 30-second
/// adapter round trip. Writing that snapshot back is a whole-node write: it
/// carried `connected_accounts` with it, so a token the refresh job rotated
/// during the probe was reverted to the value the provider had just
/// invalidated — an `invalid_grant` loop whose only visible symptom is that the
/// operator has to reconnect. The lease is held only for the read-modify-write;
/// the adapter call is long since finished.
#[cfg(feature = "storage-rocksdb")]
pub(super) async fn cache_capabilities(
    state: &crate::state::AppState,
    tenant_id: &str,
    repo: &str,
    integration_path: &str,
    caps: &Capabilities,
) -> Result<(), crate::error::ApiError> {
    use crate::error::ApiError;
    use raisin_models::nodes::properties::PropertyValue;
    let value = serde_json::to_value(caps)
        .map_err(|e| ApiError::internal(format!("failed to encode capabilities: {e}")))?;
    let pv = serde_json::from_value::<PropertyValue>(value)
        .map_err(|e| ApiError::internal(format!("failed to encode capabilities: {e}")))?;

    crate::handlers::integrations::accounts_lock::with_accounts_lock(
        state,
        tenant_id,
        repo,
        integration_path,
        "integration-test",
        move |node| {
            node.properties.insert("capabilities".to_string(), pv);
            // `Date`, not a `String` holding an RFC3339 spelling:
            // `capabilities_checked_at` is declared `Date`, and a string written
            // here would only survive because `coerce_declared_dates` rescues it
            // on the way through validation. Write the declared type at the
            // source instead of relying on the rescue.
            node.properties.insert(
                "capabilities_checked_at".to_string(),
                PropertyValue::Date(raisin_models::timestamp::StorageTimestamp::now()),
            );
            Ok(())
        },
    )
    .await
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

    use raisin_models::nodes::integrations::{build_credential, ConnectedAccount};

    fn account(id: &str) -> ConnectedAccount {
        ConnectedAccount {
            id: id.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn credential_never_contains_refresh_token() {
        let tokens = json!({ "access_token": "at-123", "refresh_token": "rt-secret" });
        let cred = build_credential("google-drive", &account("acct-1"), Some(&tokens), None, &[]);
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
        let mut acct = account("acct-1");
        acct.subject = Some("alice@example.com".to_string());
        let cred = build_credential("gmail", &acct, Some(&tokens), None, &[]);
        assert_eq!(cred.get("username").unwrap(), "alice@example.com");
        assert!(!cred.to_string().contains("rt-secret"));
    }

    /// A connection made through `Add connection` has no token blob at all —
    /// its API key lives in `secrets_encrypted`. The probe used to demand an
    /// `access_token` and so refused every one of them.
    #[test]
    fn a_connection_with_only_secrets_still_builds_a_credential() {
        let secrets = json!({ "api_key": "k-123" });
        let mut acct = account("acct-1");
        acct.config = Some(json!({ "username": "ops@example.com" }));
        let cred = build_credential(
            "http-json",
            &acct,
            None,
            Some(&secrets),
            &["username".to_string()],
        );
        assert_eq!(cred.get("api_key").unwrap(), "k-123");
        assert_eq!(cred.get("username").unwrap(), "ops@example.com");
        assert!(cred.get("access_token").is_none());
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
        let caps = capabilities_from_value(&Value::Null);
        assert!(caps.can_read);
        assert!(!caps.can_write);
        assert!(!caps.supports_changes);
    }

    /// The panel must surface the keys the sync engine carries, not a subset.
    ///
    /// This is the regression: the handler used to deserialize into its own
    /// copy of `Capabilities`, which lacked `accepts_content`, `move_fields`
    /// and `submit_unavailable_reason`, so an operator testing an IMAP outbox
    /// saw no reason for `can_submit: false`, and the cached blob lost those
    /// keys on every click.
    #[test]
    fn probe_keeps_the_late_added_capability_keys() {
        let caps = capabilities_from_value(&json!({
            "can_read": true,
            "can_write": true,
            "accepts_content": true,
            "move_fields": ["folder"],
            "submit_unavailable_reason": "email provider disabled for this tenant",
        }));
        assert!(caps.accepts_content);
        assert_eq!(caps.move_fields, vec!["folder".to_string()]);
        assert_eq!(
            caps.submit_unavailable_reason.as_deref(),
            Some("email provider disabled for this tenant")
        );
        // And they survive the round trip into the cached `capabilities` blob,
        // which is the write that used to strip them.
        let cached = serde_json::to_value(&caps).expect("encode");
        assert_eq!(cached["accepts_content"], json!(true));
        assert_eq!(cached["move_fields"], json!(["folder"]));
        assert_eq!(
            cached["submit_unavailable_reason"],
            json!("email provider disabled for this tenant")
        );
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
