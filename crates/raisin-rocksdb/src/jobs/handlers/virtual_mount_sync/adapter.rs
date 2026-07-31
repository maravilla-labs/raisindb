//! Adapter invocation: the boundary between the sync engine and adapter
//! functions.
//!
//! `raisin-rocksdb` cannot depend on `raisin-functions`, so adapters are invoked
//! through the injected [`FunctionExecutorCallback`] (the same indirection the
//! function-execution handler uses). Invocation is **direct** — never a nested
//! `FunctionExecution` job — so the sync engine never blocks a worker waiting on
//! another worker.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use thiserror::Error;

use raisin_models::nodes::integrations::{build_credential, merge_config, ConnectedAccount};

use super::materializer::MountScope;
use crate::jobs::handlers::function_execution::FunctionExecutorCallback;

/// Adapters are always stored in the "functions" workspace.
const FUNCTIONS_WORKSPACE: &str = "functions";

/// Typed engine error mapped from an adapter's thrown `code` (§4).
#[derive(Debug, Error)]
pub enum AdapterError {
    /// `code: "auth_expired"` — the account needs re-auth; pause the mount.
    #[error("adapter auth expired")]
    AuthExpired,
    /// `code: "rate_limited"` — back off and retry later.
    #[error("adapter rate limited")]
    RateLimited,
    /// `code: "conflict"` — write-through optimistic-concurrency failure.
    #[error("adapter conflict: {0}")]
    Conflict(String),
    /// Anything else — a transient failure eligible for standard job retry.
    #[error("adapter transient error: {0}")]
    Transient(String),
}

impl AdapterError {
    /// Classify an adapter failure message into a typed error. The QuickJS
    /// runtime surfaces a thrown `Error` as a message string; we match the
    /// reserved `code` values best-effort within it.
    pub fn classify(message: &str) -> Self {
        let m = message.to_ascii_lowercase();
        if m.contains("auth_expired") {
            AdapterError::AuthExpired
        } else if m.contains("rate_limited") {
            AdapterError::RateLimited
        } else if m.contains("conflict") {
            AdapterError::Conflict(message.to_string())
        } else {
            AdapterError::Transient(message.to_string())
        }
    }
}

/// Typed report of what an adapter can do, returned by its `capabilities`
/// operation. Every field defaults conservatively (`false` / `None`) so an
/// adapter that omits a field is treated as *not* supporting it. The sync engine
/// caches this on the `raisin:Integration` node so the admin UI can read it
/// without invoking the adapter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Capabilities {
    #[serde(default)]
    pub can_read: bool,
    #[serde(default)]
    pub can_write: bool,
    #[serde(default)]
    pub can_create_folders: bool,
    #[serde(default)]
    pub supports_changes: bool,
    #[serde(default)]
    pub supports_webhooks: bool,
    #[serde(default)]
    pub supports_search: bool,
    #[serde(default)]
    pub supports_push: bool,
    #[serde(default)]
    pub default_ttl: Option<u64>,
    #[serde(default)]
    pub max_file_size: Option<i64>,
}

impl Capabilities {
    /// Conservative fallback used when an adapter has no `capabilities`
    /// operation, returns a non-object, or throws: assume read-only and nothing
    /// else. This keeps the sync running (reads still work) while advertising no
    /// write/change/webhook support to the UI.
    pub fn fallback() -> Self {
        Self {
            can_read: true,
            ..Self::default()
        }
    }

    /// Parse an adapter's `capabilities` return value. Returns `None` when the
    /// value is null/absent or is not a decodable capabilities object, so the
    /// caller can substitute [`Capabilities::fallback`].
    pub fn from_adapter_value(value: &Value) -> Option<Self> {
        if value.is_null() {
            return None;
        }
        serde_json::from_value(value.clone()).ok()
    }
}

/// Result of an adapter `subscribe` operation (§ push lifecycle). The adapter
/// registers a push subscription with the provider for `params.notification_url`
/// and returns how to identify/renew it. Every field but `subscription_id` is
/// optional. `expires_at` is ISO-8601; absent means "no known expiry" (the
/// renewal job leaves such subscriptions alone).
#[derive(Debug, Clone, Deserialize)]
pub struct SubscribeResult {
    pub subscription_id: String,
    #[serde(default)]
    pub secret: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
    /// Optional provider resource identifier (opaque; stored for the adapter's
    /// own use on renew — the engine never interprets it).
    #[serde(default)]
    pub resource: Option<String>,
}

/// Result of an adapter `renew` operation: the (possibly rotated) subscription
/// id and its new ISO-8601 expiry.
#[derive(Debug, Clone, Deserialize)]
pub struct RenewResult {
    pub subscription_id: String,
    #[serde(default)]
    pub expires_at: Option<String>,
}

impl SubscribeResult {
    /// Parse an adapter `subscribe` return value. Returns `None` when the value
    /// is null or lacks a usable `subscription_id`.
    pub fn from_value(value: &Value) -> Option<Self> {
        if value.is_null() {
            return None;
        }
        serde_json::from_value(value.clone()).ok()
    }
}

impl RenewResult {
    /// Parse an adapter `renew` return value.
    pub fn from_value(value: &Value) -> Option<Self> {
        if value.is_null() {
            return None;
        }
        serde_json::from_value(value.clone()).ok()
    }
}

/// Invokes an adapter function with a fully-built §4.1 `input` object and
/// returns the operation's result value.
#[async_trait]
pub trait AdapterInvoker: Send + Sync {
    /// Invoke `adapter_path` with the pre-built `input` (`{operation, params,
    /// credential, mount}`), returning the function's return value.
    async fn invoke(
        &self,
        scope: &MountScope,
        adapter_path: &str,
        input: Value,
    ) -> Result<Value, AdapterError>;
}

/// Build the §4.1 adapter `input` object. `credential` has already had its
/// `refresh_token` stripped by the caller (see [`build_credential`]).
pub fn build_input(
    operation: &str,
    params: Value,
    credential: &Option<Value>,
    mount_snapshot: &Value,
) -> Value {
    json!({
        "operation": operation,
        "params": params,
        "credential": credential.clone().unwrap_or(Value::Null),
        "mount": mount_snapshot.clone(),
    })
}

/// Build the read-only `mount` snapshot passed to adapters.
///
/// The full authored `sync_config` is forwarded verbatim (not a whitelist) so
/// adapters receive every provider-specific key, and `api_config` is attached
/// so legacy connection settings (host/port/tls/mailbox/auth) reach the adapter
/// as well. Both are byte-identical to what they have always been — adapters
/// that read them keep working untouched.
///
/// `config` is the additional, resolved view: `api_config` < connector `config`
/// < connection `config` < `sync_config`, merged per top-level key. New adapters
/// should read `config` and ignore the other two.
pub fn build_mount_snapshot(
    mount: &super::config::MountConfig,
    integration: &super::config::IntegrationConfig,
    account: Option<&ConnectedAccount>,
) -> Value {
    let account_config = account.map(|a| a.config_or_empty()).unwrap_or(Value::Null);
    let merged = merge_config(
        &integration.api_config,
        &integration.config,
        &account_config,
        &mount.sync_config_raw,
    );
    json!({
        "mount_id": mount.mount_id,
        "remote_root": mount.remote_root,
        "mount_path": mount.mount_path,
        "sync_config": mount.sync_config_raw,
        "api_config": integration.api_config,
        "config": merged,
    })
}

/// Decrypt a mount's chosen connection into an adapter credential.
///
/// Handles both connection shapes: OAuth (a `tokens_encrypted` blob) and
/// credential-based (a `secrets_encrypted` map — an IMAP app password). The
/// previous implementation returned `None` the moment `tokens_encrypted` was
/// absent, which is precisely why a password-only connection could never work.
///
/// Returns `None` only when no connection can be resolved or no master key is
/// configured. Shared by the sync run, the push-subscription lifecycle and the
/// renewal scan so every adapter invocation builds the credential identically.
pub fn build_mount_credential(
    mount: &super::config::MountConfig,
    integration: &super::config::IntegrationConfig,
) -> Option<Value> {
    let account = integration.account_for(mount.account_ref.as_deref()).ok()?;
    let key = raisin_crypto::master_key_with_embedding_fallback()
        .ok()
        .flatten()?;
    let secret_box = raisin_crypto::SecretBox::new(&key);

    // Either half may be absent; a connection with neither is unusable.
    let tokens = account
        .tokens_encrypted
        .as_deref()
        .and_then(|enc| secret_box.decrypt_json(enc).ok());
    let secrets = account
        .secrets_encrypted
        .as_deref()
        .and_then(|enc| secret_box.decrypt_json(enc).ok());
    if tokens.is_none() && secrets.is_none() {
        return None;
    }

    Some(build_credential(
        &integration.provider_type,
        account,
        tokens.as_ref(),
        secrets.as_ref(),
        &integration.credential_fields,
    ))
}

/// Production [`AdapterInvoker`] backed by the injected function executor.
pub struct FunctionAdapterInvoker {
    executor: FunctionExecutorCallback,
}

impl FunctionAdapterInvoker {
    /// Wrap a function executor callback.
    pub fn new(executor: FunctionExecutorCallback) -> Self {
        Self { executor }
    }
}

#[async_trait]
impl AdapterInvoker for FunctionAdapterInvoker {
    async fn invoke(
        &self,
        scope: &MountScope,
        adapter_path: &str,
        input: Value,
    ) -> Result<Value, AdapterError> {
        let execution_id = format!("vmount-{}", nanoid::nanoid!());
        let fut = (self.executor)(
            adapter_path.to_string(),
            execution_id,
            input,
            scope.tenant.clone(),
            scope.repo.clone(),
            scope.branch.clone(),
            FUNCTIONS_WORKSPACE.to_string(),
            None, // system context: adapters run privileged (admin-installed)
            None, // no live log streaming for sync invocations
        );
        let result = fut
            .await
            .map_err(|e| AdapterError::Transient(format!("adapter invocation failed: {e}")))?;

        if result.success {
            Ok(result.result.unwrap_or(Value::Null))
        } else {
            let msg = result
                .error
                .unwrap_or_else(|| "adapter returned failure without a message".to_string());
            Err(AdapterError::classify(&msg))
        }
    }
}

/// Shared handle to an adapter invoker.
pub type AdapterInvokerHandle = Arc<dyn AdapterInvoker>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_maps_reserved_codes() {
        assert!(matches!(
            AdapterError::classify("Error: auth_expired"),
            AdapterError::AuthExpired
        ));
        assert!(matches!(
            AdapterError::classify("rate_limited by provider"),
            AdapterError::RateLimited
        ));
        assert!(matches!(
            AdapterError::classify("etag conflict"),
            AdapterError::Conflict(_)
        ));
        assert!(matches!(
            AdapterError::classify("boom"),
            AdapterError::Transient(_)
        ));
    }

    #[test]
    fn credential_never_contains_refresh_token() {
        // The exhaustive rules live with the builder in raisin-models; this
        // pins the engine's own wiring of it.
        let account = ConnectedAccount {
            id: "acct1".to_string(),
            ..Default::default()
        };
        let tokens = json!({ "access_token": "at", "refresh_token": "rt" });
        let cred = build_credential("google-drive", &account, Some(&tokens), None, &[]);
        assert_eq!(cred.get("access_token").unwrap(), "at");
        assert!(cred.get("refresh_token").is_none());
        assert!(cred.get("username").is_none());
        assert_eq!(cred.get("provider_type").unwrap(), "google-drive");
    }
}
