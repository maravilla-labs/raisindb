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
pub(super) fn adapter_error_code(msg: &str) -> &'static str {
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
pub(super) fn sanitize(msg: &str) -> String {
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
pub(super) fn resolve_credential(
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

/// Minimal read-only `mount` snapshot for a one-off probe. Forwards the
/// integration's `api_config` (where connectors like IMAP/Gmail keep host/port/
/// tls/auth) so the probe reaches the provider the same way a real sync would.
#[cfg(feature = "storage-rocksdb")]
pub(super) fn mount_snapshot(remote_root: &Option<String>, api_config: &Value) -> Value {
    json!({
        "mount_id": "connection-test",
        "remote_root": remote_root,
        "mount_path": "/",
        "api_config": api_config,
        "sync_config": {
            "mode": "poll",
            "interval_seconds": 300,
            "include_patterns": [],
            "exclude_patterns": [],
            "ephemeral": false,
            "ttl_seconds": null,
            "max_items_per_sync": PROBE_LIMIT,
        },
    })
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

    #[test]
    fn sanitize_redacts_tokens() {
        let s = sanitize("failed with ya29.abcdef and bearer xyz");
        assert!(!s.contains("ya29"));
        assert!(s.contains("[redacted]"));
    }
}
