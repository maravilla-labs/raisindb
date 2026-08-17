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
    ///
    /// Carries the provider's own `Retry-After` when it stated one. Guessing
    /// instead is worse in both directions: too short re-hammers a throttled
    /// tenant (the self-sustaining spiral that killed a production calendar
    /// walk), too long stalls a mount that was told it could resume in twenty
    /// seconds. `None` means the provider said nothing and the engine's
    /// exponential backoff applies.
    #[error("adapter rate limited")]
    RateLimited { retry_after_secs: Option<u64> },
    /// `code: "conflict"` — write-through optimistic-concurrency failure.
    #[error("adapter conflict: {0}")]
    Conflict(String),
    /// `code: "config_error"` — the mount or connector is misconfigured and the
    /// request can NEVER succeed as written (a malformed remote root, a folder
    /// that does not exist, an unsupported resource).
    ///
    /// Distinct from [`Transient`] because retrying is not merely useless, it is
    /// harmful: a mount with a bad remote root retried a Graph call that always
    /// answered "Id is malformed", three times per job, on every scheduler tick,
    /// indefinitely — which is how an OAuth app gets throttled. Treated as a
    /// terminal outcome for the run: the state records it and the job returns
    /// success, so the job layer does not retry on top of the scheduler.
    #[error("adapter configuration error: {0}")]
    Config(String),
    /// `code: "cursor_invalid"` — the stored delta cursor is no longer accepted
    /// by the provider and the mount must resynchronize from a full walk.
    ///
    /// Every incremental API expires its cursors: Graph answers `410 Gone` with
    /// `syncStateNotFound` / `resyncRequired`, Google returns `410` on a stale
    /// `syncToken`, IMAP invalidates on `UIDVALIDITY` change. The request can
    /// never succeed as sent, so this is NOT transient — but unlike [`Config`]
    /// it needs no operator action, because the engine can recover by itself:
    /// drop the cursor and fall back to a full reconcile.
    ///
    /// Classifying this as `Transient` is what left a production mount wedged.
    /// Graph rejected a `generation=1` token while it had moved to generation
    /// 51; the job retried three times per tick forever, never succeeded, and
    /// the resulting `consecutive_failures` also gated the backfill fast path in
    /// `check::is_due` — so the pending import could not drain either.
    #[error("adapter cursor invalid: {0}")]
    CursorInvalid(String),
    /// Anything else — a transient failure eligible for standard job retry.
    #[error("adapter transient error: {0}")]
    Transient(String),
}

/// Pull `retry_after=<seconds>` out of an adapter's error message.
///
/// The QuickJS host surfaces a thrown `Error` as its message string and
/// nothing else, so a structured field cannot cross the boundary — the
/// adapter's `coded()` helper appends the value to the text and this reads it
/// back. Absent, unparseable or non-positive values yield `None`, which simply
/// means "the provider did not say" and leaves the exponential backoff in
/// charge.
fn parse_retry_after(lowercased_message: &str) -> Option<u64> {
    let idx = lowercased_message.find("retry_after=")?;
    let rest = &lowercased_message[idx + "retry_after=".len()..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let secs: u64 = digits.parse().ok()?;
    // A provider asking us to wait a week is a bug or a hostile answer, not an
    // instruction to take a mount offline for a week. Cap it at an hour; the
    // scheduler's own interval takes over from there.
    const MAX_RETRY_AFTER_SECS: u64 = 3600;
    (secs > 0).then_some(secs.min(MAX_RETRY_AFTER_SECS))
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
            AdapterError::RateLimited {
                retry_after_secs: parse_retry_after(&m),
            }
        } else if m.contains("cursor_invalid") {
            AdapterError::CursorInvalid(message.to_string())
        } else if m.contains("config_error") {
            AdapterError::Config(message.to_string())
        } else if m.contains("conflict") {
            AdapterError::Conflict(message.to_string())
        } else {
            AdapterError::Transient(message.to_string())
        }
    }

    /// The provider's requested wait, when it stated one.
    pub fn retry_after_secs(&self) -> Option<u64> {
        match self {
            AdapterError::RateLimited { retry_after_secs } => *retry_after_secs,
            _ => None,
        }
    }

    /// Whether re-running the identical request could plausibly succeed.
    ///
    /// The single place this judgement is made, so the scheduler, the job-retry
    /// decision and the mount status can never disagree about an error.
    pub fn is_retryable(&self) -> bool {
        match self {
            AdapterError::RateLimited { .. } | AdapterError::Transient(_) => true,
            // AuthExpired pauses the mount until reconnect; Config needs an
            // operator edit; Conflict is resolved by the next sync's fresh read.
            // CursorInvalid is recovered in-run by dropping the cursor and
            // doing a full reconcile, so a job-level retry of the identical
            // delta request would only repeat a request the provider has
            // already refused.
            AdapterError::AuthExpired
            | AdapterError::Config(_)
            | AdapterError::Conflict(_)
            | AdapterError::CursorInvalid(_) => false,
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
    /// Adapter implements the optional `browse` operation (§2.10). The engine
    /// never calls `browse` — this is carried so the cached capabilities the UI
    /// reads are complete; dropping it here would silently strip the flag from
    /// every integration node the engine writes.
    #[serde(default)]
    pub supports_browse: bool,
    #[serde(default)]
    pub default_ttl: Option<u64>,
    #[serde(default)]
    pub max_file_size: Option<i64>,

    // ---- write path (§3.3 / §10 of docs/reference/virtual-node-adapters.md) ----
    //
    // All optional, all `false` / empty by default, so an adapter written before
    // the write path existed — i.e. every adapter shipped today — is correctly
    // reported as read-only rather than accidentally write-enabled.
    /// Adapter implements the `create` operation (a local create propagates).
    #[serde(default)]
    pub can_create: bool,
    /// Adapter implements the `update` operation (a local edit propagates).
    #[serde(default)]
    pub can_update: bool,
    /// Adapter implements the `delete` operation (a local delete propagates,
    /// subject to the mount's `delete_policy`).
    #[serde(default)]
    pub can_delete: bool,
    /// Adapter implements the `submit` operation — issuing a command (send a
    /// mail, RSVP) rather than mirroring an object.
    #[serde(default)]
    pub can_submit: bool,
    /// The `state_only` allow-list: which node properties this provider accepts
    /// as writes. The engine has no domain knowledge (it does not know a mail
    /// body is immutable while its read flag is not), so this is how a provider
    /// explains what "writable" means for it. Empty means "nothing declared".
    #[serde(default)]
    pub mutable_fields: Vec<String>,
    /// Which of [`Self::mutable_fields`] express the object's LOCATION at the
    /// provider — the folder, the parent, the label set that IS the mailbox.
    ///
    /// A move is an `update` carrying this field and nothing more (§8), so this
    /// is what lets `move_policy` mean anything: the engine is domain-blind and
    /// cannot tell that `folder` relocates a message while `unread` does not.
    /// Empty — the default, and where every adapter shipped today is — means no
    /// declared field relocates anything, so `move_policy` has nothing to gate
    /// and the mount behaves exactly as it did before this existed.
    ///
    /// A name here that is NOT in `mutable_fields` is inert: the effective push
    /// list is the intersection of mount and adapter, and a field the adapter
    /// does not accept as a write never reaches the classification at all.
    #[serde(default)]
    pub move_fields: Vec<String>,
    /// Recommended default for a local delete: `"detach" | "trash" | "purge"`.
    /// A mount may override it; `purge` is never a default.
    #[serde(default)]
    pub default_delete_policy: Option<String>,
    /// Recommended default for a local move: `"push" | "detach" | "reject"`.
    #[serde(default)]
    pub default_move_policy: Option<String>,
    /// `delete` can soft-delete (provider trash) rather than purge.
    #[serde(default)]
    pub supports_trash: bool,
    /// `submit` can forward a provider-side idempotency key, so the engine's
    /// at-most-once attempt id is honoured end to end.
    #[serde(default)]
    pub supports_idempotency_key: bool,
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

    /// Which of the operations a `mirror` write mode needs this adapter does
    /// NOT declare, in a stable order suitable for an operator-facing message.
    ///
    /// `mirror` means "the node **is** the remote object", so create, update and
    /// delete all have to propagate; `can_write` is the umbrella flag an adapter
    /// sets to say it writes at all. Empty means every needed op is declared.
    ///
    /// `needs_create` comes from the MOUNT, not the adapter: the engine issues
    /// `create` only for a mount that opted a node type into local creation, so
    /// an adapter that updates and deletes is a complete mirror for every mount
    /// that does not. Demanding the capability unconditionally refused correct
    /// adapters for an op they would never be called with.
    pub fn missing_mirror_ops(
        &self,
        needs_create: bool,
        propagates_deletes: bool,
    ) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if !self.can_write {
            missing.push("can_write");
        }
        if needs_create && !self.can_create {
            missing.push("can_create");
        }
        if !self.can_update {
            missing.push("can_update");
        }
        // Only from a mount that actually deletes upstream. A `detach` mount
        // unhooks the node locally and never calls the adapter, so demanding
        // the capability refused mounts for an operation they are configured
        // never to perform — see the note at the call site in `resolve_mirror`.
        if propagates_deletes && !self.can_delete {
            missing.push("can_delete");
        }
        missing
    }

    /// Which of the operations a `state_only` write mode needs this adapter
    /// does NOT declare, in a stable order suitable for an operator message.
    ///
    /// `state_only` pushes a declared allow-list of fields onto an existing
    /// remote object, so it needs `update` and nothing else — no create, no
    /// delete. Reusing [`Self::missing_mirror_ops`] here would refuse a perfectly
    /// good mail mount for lacking a `delete` it will never call.
    pub fn missing_state_only_ops(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if !self.can_write {
            missing.push("can_write");
        }
        if !self.can_update {
            missing.push("can_update");
        }
        missing
    }

    /// Which of the operations a `submit` write mode needs this adapter does
    /// NOT declare.
    ///
    /// `submit` issues a COMMAND — it neither mirrors nor patches a remote
    /// object — so `can_update`, `can_create` and `can_delete` say nothing about
    /// it and requiring them would refuse a perfectly good outbox for lacking
    /// operations it will never call. `can_write` is still required: it is the
    /// umbrella flag that says this adapter changes anything at the provider at
    /// all, and an adapter that sets `can_submit` without it has contradicted
    /// itself rather than opted in.
    ///
    /// Note what is NOT here: [`Self::supports_idempotency_key`]. An adapter
    /// that cannot forward a provider-side key is still a usable outbox — the
    /// engine's at-most-once protocol is built on the durable `queued ->
    /// sending` claim, not on the provider cooperating, precisely because
    /// almost none of them do (Graph's `sendMail` has no such key, and SMTP has
    /// nothing at all). Requiring it would refuse every real adapter.
    pub fn missing_submit_ops(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if !self.can_write {
            missing.push("can_write");
        }
        if !self.can_submit {
            missing.push("can_submit");
        }
        missing
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

    /// A provider that states how long to wait must be obeyed, not guessed at.
    #[test]
    fn a_stated_retry_after_survives_the_js_boundary() {
        // The adapter's `coded()` appends it; only the message string crosses.
        let e = AdapterError::classify(
            "get_changes: Microsoft Graph is busy (503) (retry_after=120): rate_limited",
        );
        assert!(matches!(
            e,
            AdapterError::RateLimited {
                retry_after_secs: Some(120)
            }
        ));
        assert_eq!(e.retry_after_secs(), Some(120));

        // Silence means "the provider did not say" — the engine's exponential
        // backoff stays in charge, which is NOT the same as "retry now".
        let quiet = AdapterError::classify("rate_limited");
        assert!(matches!(
            quiet,
            AdapterError::RateLimited {
                retry_after_secs: None
            }
        ));

        // A hostile or buggy value must not take a mount offline for a week.
        assert_eq!(
            AdapterError::classify("rate_limited (retry_after=999999)").retry_after_secs(),
            Some(3600)
        );
        // Junk parses as "not stated" rather than as a wrong number.
        assert_eq!(
            AdapterError::classify("rate_limited (retry_after=soon)").retry_after_secs(),
            None
        );
        assert_eq!(
            AdapterError::classify("rate_limited (retry_after=0)").retry_after_secs(),
            None
        );
        // Only rate limits carry one.
        assert_eq!(
            AdapterError::classify("config_error (retry_after=30)").retry_after_secs(),
            None
        );
    }

    #[test]
    fn classify_maps_reserved_codes() {
        assert!(matches!(
            AdapterError::classify("Error: auth_expired"),
            AdapterError::AuthExpired
        ));
        assert!(matches!(
            AdapterError::classify("rate_limited by provider"),
            AdapterError::RateLimited { .. }
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

    /// An expired delta cursor must be recoverable, not retried.
    ///
    /// Verbatim production message: Graph rejected a `generation=1` delta token
    /// after it had moved to generation 51. Classified as `Transient` it was
    /// retried three times per scheduler tick indefinitely — the identical
    /// rejected cursor every time — and the failure counter it accumulated also
    /// blocked the mount's pending backfill from draining.
    #[test]
    fn expired_delta_cursor_is_recoverable_not_retryable() {
        let msg = "get_changes: The sync state generation is not found; \
                   generation=1;[highest=51][49][50][51]. cursor_invalid";
        let err = AdapterError::classify(msg);
        assert!(matches!(err, AdapterError::CursorInvalid(_)));
        assert!(
            !err.is_retryable(),
            "retrying re-sends the cursor the provider already refused"
        );
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
