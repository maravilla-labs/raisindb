//! Generic push/webhook subscription lifecycle.
//!
//! A push notification is treated as *only* an invalidation signal: the engine
//! never inspects a provider payload. All the engine does is register a
//! subscription (adapter `subscribe`), keep it alive (adapter `renew`), and tear
//! it down (adapter `unsubscribe`). When a provider later pings the public
//! notifications endpoint, that endpoint simply enqueues the mount's normal
//! delta sync — the mount's existing `get_changes` does the work.
//!
//! Nothing here is provider-specific: Graph subscriptions, Google Calendar
//! channels, and Gmail Pub/Sub all collapse to the same three operations. Every
//! provider quirk (validation handshakes, header names, payload shapes) lives in
//! adapter JS, never in the engine.

use std::sync::Arc;

use chrono::Utc;
use serde_json::{json, Value};

use super::adapter::{AdapterError, AdapterInvoker, RenewResult, SubscribeResult};
use super::config::{MountConfig, MountState};
use super::materializer::{MountScope, NodeMaterializer};
use super::{Capabilities, SyncCtx};
use crate::RocksDBStorage;

/// Renew subscriptions that expire within this window (1 day). Provider
/// subscriptions are short-lived (Graph ~3d, Google ~7d), so a daily-headroom
/// renew keeps them alive with margin.
pub const RENEW_WINDOW_SECS: i64 = 24 * 60 * 60;

/// The configured canonical base URL (`RAISINDB_BASE_URL`), trailing slash
/// trimmed, or `None` when unset/empty. The engine cannot depend on
/// `raisin-transport-http`, so it reads the same env var that transport's
/// `configured_base_url` reads (kept in sync deliberately). When unset, push
/// cannot be wired (there is no public URL to hand the provider) and the mount
/// is marked `failed` with an explanatory error.
pub fn configured_base_url() -> Option<String> {
    std::env::var("RAISINDB_BASE_URL")
        .ok()
        .map(|b| b.trim_end_matches('/').to_string())
        .filter(|b| !b.is_empty())
}

/// Build the per-mount notifications URL handed to the provider on `subscribe`.
/// Shape (frozen — the HTTP notifications endpoint parses it):
/// `{base}/api/integrations/{repo}/notifications/{mount_token}`.
pub fn notification_url(base: &str, repo: &str, mount_token: &str) -> String {
    format!("{base}/api/integrations/{repo}/notifications/{mount_token}")
}

/// Ensure the mount has a stable, unguessable `push_mount_token`, generating one
/// on first use and persisting it into `state`. Format: `"{mount_id}.{nanoid}"`.
///
/// The endpoint splits on the LAST `.` to recover `mount_id` for an O(1) node
/// lookup, then constant-time compares the whole token against the stored value.
/// The random suffix (URL-safe nanoid, 32 chars) is what makes the URL
/// unguessable; the `mount_id` prefix only makes lookup direct.
pub fn ensure_mount_token(mount_id: &str, state: &mut MountState) -> String {
    if let Some(tok) = &state.push_mount_token {
        return tok.clone();
    }
    let tok = format!("{mount_id}.{}", nanoid::nanoid!(32));
    state.push_mount_token = Some(tok.clone());
    tok
}

/// Build a throwaway [`SyncCtx`] for a non-materializing adapter call
/// (subscribe/renew/unsubscribe). No lease is taken (`lock_manager: None`,
/// empty `lock_key`) — these calls are short and idempotent.
#[allow(clippy::too_many_arguments)]
pub fn adapter_ctx<'a>(
    storage: Arc<RocksDBStorage>,
    scope: MountScope,
    config_branch: String,
    mount: MountConfig,
    adapter_path: String,
    invoker: &'a dyn AdapterInvoker,
    materializer: &'a dyn NodeMaterializer,
    credential: Option<Value>,
    mount_snapshot: Value,
    public_origin: Option<String>,
) -> SyncCtx<'a> {
    SyncCtx {
        // Subscription maintenance never comes from a delivery, so there is nothing to
        // apply — and it does not call `get_changes` at all.
        pushed_events: None,
        public_origin,
        storage,
        scope,
        config_branch,
        mount,
        adapter_path,
        invoker,
        materializer,
        lock_manager: None,
        lock_key: String::new(),
        // Deliberately unleased: subscribe/renew/teardown run outside the
        // per-mount sync lease (see the module docs), so there is no handle to
        // renew with.
        lease_token: None,
        credential,
        mount_snapshot,
        // A subscription op is a single adapter call, not a walk, so the
        // budget never applies. Far-future so nothing can read it as expired.
        deadline: i64::MAX,
        // Subscription contexts never drain.
        // Subscription management never pushes a node, so it needs no byte
        // channel — this context exists to call subscribe/renew/unsubscribe.
        binary_retrieval: None,
        write_mode: std::sync::OnceLock::new(),
    }
}

/// Ensure a push subscription exists for a mount that wants push, mutating
/// `state` in place (persisted by the caller's normal state write). Idempotent:
/// a mount that already has a live subscription is left untouched.
///
/// Provider-agnostic policy:
/// - mode `poll`, or `supports_push == false` → never subscribe.
/// - a webhook-mode mount whose adapter can't push is marked `unsupported` so the
///   periodic check stops trying to bootstrap it (it will neither poll nor push
///   — a misconfiguration the admin must fix; a one-time warning is logged).
/// - otherwise, if there is no active non-expired subscription, call `subscribe`
///   and record the returned `{subscription_id, secret, expires_at}`.
pub async fn ensure(ctx: &SyncCtx<'_>, state: &mut MountState, caps: &Capabilities) {
    let mode = ctx.mount.sync_config.mode.as_str();
    if mode != "webhook" && mode != "hybrid" {
        return;
    }
    let mount_id = &ctx.scope.mount_id;

    if !caps.supports_push {
        if mode == "webhook" && state.push_status.as_deref() != Some("unsupported") {
            state.push_status = Some("unsupported".to_string());
            state.push_last_error = Some(
                "adapter does not support push (capabilities.supports_push = false)".to_string(),
            );
            tracing::warn!(
                mount_id = %mount_id,
                "webhook-mode mount's adapter does not support push; it will neither poll \
                 nor receive pushes until reconfigured"
            );
        }
        return;
    }

    let now = Utc::now().timestamp();
    if state.has_active_push(now) {
        return; // idempotent: a live subscription already exists
    }

    // Prefer the connector's own public origin, derived from the operator's
    // stored OAuth redirect_uri, and fall back to the global env var.
    //
    // `RAISINDB_BASE_URL` cannot express a multi-tenant deployment: every org is
    // served at its own `{handle}.{base}` host, so one static value is wrong for
    // all but one of them and the var is simply left unset — which meant push
    // could never be wired at all. The redirect_uri is the one public URL per
    // connector the operator already has to get right, because the provider
    // rejects an OAuth exchange that does not match what is registered. Reusing
    // it means no second thing to configure and no second thing to get wrong.
    let Some(base) = ctx.public_origin.clone().or_else(configured_base_url) else {
        state.push_status = Some("failed".to_string());
        state.push_last_error = Some(
            "no public URL for this connector: set the OAuth Redirect URI on the connector (its \
             origin is reused for push notifications), or set RAISINDB_BASE_URL"
                .to_string(),
        );
        tracing::warn!(
            mount_id = %mount_id,
            "cannot wire push: no connector redirect_uri origin and RAISINDB_BASE_URL unset"
        );
        return;
    };

    let token = ensure_mount_token(mount_id, state);
    let url = notification_url(&base, &ctx.scope.repo, &token);

    match ctx
        .call("subscribe", json!({ "notification_url": url }))
        .await
    {
        Ok(v) => match SubscribeResult::from_value(&v) {
            Some(r) => {
                state.push_subscription_id = Some(r.subscription_id);
                state.push_secret = r.secret;
                state.push_expires_at = r.expires_at;
                state.push_notification_url = Some(url);
                state.push_status = Some("active".to_string());
                state.push_last_error = None;
                tracing::info!(mount_id = %mount_id, "push subscription registered");
            }
            None => {
                state.push_status = Some("failed".to_string());
                state.push_last_error = Some("subscribe returned no subscription_id".to_string());
                tracing::warn!(mount_id = %mount_id, "adapter subscribe returned no subscription_id");
            }
        },
        Err(e) => {
            state.push_status = Some("failed".to_string());
            state.push_last_error = Some(e.to_string());
            tracing::warn!(mount_id = %mount_id, error = %e, "adapter subscribe failed");
        }
    }
}

/// Best-effort teardown: unsubscribe the provider subscription (ignoring
/// errors — the provider will expire it anyway) and clear the push fields from
/// `state`. The stable `push_mount_token` is retained so a later re-enable
/// reuses the same notification URL.
pub async fn teardown(ctx: &SyncCtx<'_>, state: &mut MountState) {
    let Some(sub) = state.push_subscription_id.clone() else {
        return;
    };
    if let Err(e) = ctx
        .call("unsubscribe", json!({ "subscription_id": sub }))
        .await
    {
        tracing::debug!(
            mount_id = %ctx.scope.mount_id,
            error = %e,
            "adapter unsubscribe failed (ignored; provider will expire it)"
        );
    }
    state.push_subscription_id = None;
    state.push_secret = None;
    state.push_expires_at = None;
    state.push_notification_url = None;
    state.push_status = None;
    state.push_last_error = None;
}

/// Renew one mount's subscription in place. Returns `true` when the subscription
/// was successfully renewed. On failure the mount is marked `failed` (so the
/// periodic check re-bootstraps a fresh subscription via [`ensure`]).
pub async fn renew(ctx: &SyncCtx<'_>, state: &mut MountState) -> bool {
    let Some(sub) = state.push_subscription_id.clone() else {
        return false;
    };
    let url = state.push_notification_url.clone().unwrap_or_default();
    match ctx
        .call(
            "renew",
            json!({ "subscription_id": sub, "notification_url": url }),
        )
        .await
    {
        Ok(v) => match RenewResult::from_value(&v) {
            Some(r) => {
                state.push_subscription_id = Some(r.subscription_id);
                if r.expires_at.is_some() {
                    state.push_expires_at = r.expires_at;
                }
                state.push_status = Some("active".to_string());
                state.push_last_error = None;
                tracing::info!(mount_id = %ctx.scope.mount_id, "push subscription renewed");
                true
            }
            None => {
                mark_renew_failed(state, "renew returned no subscription_id");
                tracing::warn!(mount_id = %ctx.scope.mount_id, "adapter renew returned no subscription_id");
                false
            }
        },
        Err(AdapterError::AuthExpired) => {
            mark_renew_failed(state, "auth_expired");
            false
        }
        Err(e) => {
            mark_renew_failed(state, &e.to_string());
            tracing::warn!(mount_id = %ctx.scope.mount_id, error = %e, "adapter renew failed");
            false
        }
    }
}

fn mark_renew_failed(state: &mut MountState, err: &str) {
    state.push_status = Some("failed".to_string());
    state.push_last_error = Some(err.to_string());
}
