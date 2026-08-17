// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! `POST /management/integrations/connect-event` — a control-plane invalidation kick.
//!
//! # Why this exists rather than a per-mount notification
//!
//! Some providers do not let a platform register a webhook per connection. Stripe Connect
//! is the case that forced this: a platform gets exactly ONE endpoint for all of its
//! connected accounts, and each delivered event names its source account. There is no
//! per-mount subscription to create, so the per-mount
//! `/api/integrations/{repo}/notifications/{mount_token}` door cannot be used.
//!
//! The control plane (maravilla-connect) receives that single webhook, verifies its
//! signature, resolves the account to the tenants that consented to it — a mapping only it
//! has, because it issued the grants — and calls here once per tenant. This handler does
//! the last hop: tenant → repos with mounts → the mounts on that account → a delta sync.
//!
//! # Why the tenant comes from the header
//!
//! `X-Tenant-Id` is authoritative here because the caller is the control plane, holding
//! the superadmin bearer, deliberately naming which tenant it means. That is the opposite
//! of a public route, where trusting the header would be the bug: `resolve_tenant_id`
//! falls back to `"default"` when it is absent, so an unauthenticated caller could aim a
//! request at the wrong tenant simply by omitting it.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::json;

use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ConnectEventRequest {
    /// Which provider the event came from, e.g. `stripe`.
    pub provider: String,
    /// The connected account the event belongs to, e.g. `acct_…`.
    pub account: String,
    /// For logging only — this handler never branches on it. The control plane has
    /// already decided the event is worth acting on, and a delta sync re-reads the
    /// resource regardless of which event prompted it.
    #[serde(default)]
    pub event_type: String,
    #[serde(default)]
    pub event_id: String,
}

/// Fan one provider event out to the mounts that care about it.
///
/// Always answers 200 on a well-formed request, including when nothing matches. The
/// caller must not read "this tenant has no mount for that account" as a failure: the
/// account may be linked to a different tenant, or connected but not yet mounted. Turning
/// that into an error would make the control plane retry, and — for a provider that
/// disables endpoints on repeated failure — eventually cost every tenant their push.
#[cfg(feature = "storage-rocksdb")]
pub async fn connect_event(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantInfo>,
    Json(body): Json<ConnectEventRequest>,
) -> Result<Response, ApiError> {
    let Some(storage) = state.rocksdb_storage() else {
        return Err(ApiError::internal("RocksDB backend required"));
    };

    let repos =
        raisin_rocksdb::management::list_repos_with_virtual_mounts(storage, &tenant.tenant_id)
            .await
            .map_err(|e| ApiError::internal(format!("failed to list repos with mounts: {e}")))?;

    let mut matched = 0usize;
    let mut queued = 0usize;

    for repo in repos {
        let svc = super::config_service(&state, &tenant.tenant_id, &repo, "connect-event");

        // Match the ACCOUNT first, on the integration nodes. A repo that has never
        // connected this account is skipped without reading a single mount — which is
        // what keeps this cheap on a tenant with many repos and one Stripe connection.
        let integrations = match svc.list_by_type("raisin:Integration").await {
            Ok(nodes) => nodes,
            Err(e) => {
                tracing::warn!(repo = %repo, error = %e, "connect-event: cannot list integrations");
                continue;
            }
        };

        let mut account_ids = Vec::new();
        for node in &integrations {
            let Ok(config) = raisin_models::nodes::integrations::IntegrationConfig::from_node(node)
            else {
                continue;
            };
            if config.provider_type != body.provider {
                continue;
            }
            for account in &config.accounts {
                if account.subject.as_deref() == Some(body.account.as_str()) {
                    account_ids.push(account.id.clone());
                }
            }
        }
        if account_ids.is_empty() {
            continue;
        }

        let mounts = match svc.list_by_type("raisin:VirtualMount").await {
            Ok(nodes) => nodes,
            Err(e) => {
                tracing::warn!(repo = %repo, error = %e, "connect-event: cannot list mounts");
                continue;
            }
        };

        // Resolved once per repo, not once per mount, and deliberately the SAME resolution
        // the manual-sync path uses. `config_service` above reads on `CONFIG_BRANCH` while
        // the sync engine reads mount config from the repo's real default branch; the two
        // can differ, and using one resolution for the read and another for the job is the
        // documented cause of a sync that finds no mount and fails silently.
        let branch = super::config_branch(&state, &tenant.tenant_id, &repo).await;

        for mount in mounts {
            if !mount_wants_push(&mount, &account_ids) {
                continue;
            }
            matched += 1;

            match super::notifications::enqueue_delta_sync(
                &state,
                &tenant.tenant_id,
                &repo,
                &branch,
                &mount.id,
            )
            .await
            {
                // `false` means a sync for this mount is already in flight — the
                // idempotency key collapsed it. That is a success, not a miss: a burst of
                // events for one account should cost one sync.
                Ok(true) => {
                    queued += 1;
                    record_delivery(&state, &tenant.tenant_id, &repo, &mount.id).await;
                }
                Ok(false) => {
                    record_delivery(&state, &tenant.tenant_id, &repo, &mount.id).await;
                }
                Err(e) => {
                    tracing::warn!(
                        mount_id = %mount.id, repo = %repo, error = %e,
                        "connect-event: could not enqueue a delta sync"
                    );
                }
            }
        }
    }

    if matched == 0 {
        // Info, not warn. The common shape is a tenant that connected the account but has
        // not mounted anything from it yet.
        tracing::info!(
            tenant = %tenant.tenant_id, provider = %body.provider,
            account = %body.account, event_type = %body.event_type,
            "connect-event matched no mount"
        );
    }

    Ok((
        StatusCode::OK,
        Json(json!({ "matched": matched, "queued": queued })),
    )
        .into_response())
}

/// Should this mount be woken by a push for one of `account_ids`?
#[cfg(feature = "storage-rocksdb")]
fn mount_wants_push(mount: &raisin_models::nodes::Node, account_ids: &[String]) -> bool {
    use super::json_prop;
    wants_push(
        &json_prop(mount, "enabled"),
        &json_prop(mount, "state"),
        &json_prop(mount, "sync_config"),
        &json_prop(mount, "account_ref"),
        account_ids,
    )
}

/// The decision itself, over plain JSON.
///
/// Split from the node read so it can be tested directly — the interesting logic is
/// entirely in these four values, and building a `Node` to exercise it would test the
/// node builder rather than the rule.
///
/// Three separate reasons to skip, all of which mean the operator asked for this:
/// a disabled mount, a paused one, and a `poll`-mode mount that never opted into push.
fn wants_push(
    enabled: &serde_json::Value,
    state: &serde_json::Value,
    sync_config: &serde_json::Value,
    account_ref: &serde_json::Value,
    account_ids: &[String],
) -> bool {
    if enabled == &serde_json::Value::Bool(false) {
        return false;
    }
    if state.get("paused").and_then(|v| v.as_bool()) == Some(true) {
        return false;
    }

    // `poll` is an explicit statement that this mount is driven by its interval. Waking it
    // on push would silently override the operator's choice. An ABSENT mode is `poll`,
    // matching the engine's own default.
    let mode = sync_config
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("poll");
    if mode != "webhook" && mode != "hybrid" {
        return false;
    }

    // An explicit account_ref must name one of the matched accounts. An ABSENT one means
    // "this repo's single connection" — safe to treat as a match only because the caller
    // already established the account is connected in this repo. Never fall back to "the
    // first account": on a repo with two connections that silently syncs the wrong one,
    // which the integration config layer records having shipped once.
    match account_ref.as_str() {
        Some(explicit) => account_ids.iter().any(|id| id == explicit),
        None => true,
    }
}

/// Stamp delivery health so the console does not report "no push" for a mount that is
/// demonstrably being pushed.
///
/// Uses `CONFIG_BRANCH` — the branch the mount node was READ on — rather than the job's
/// branch. Mixing the two is what left production showing "ACCEPTED 0" while notifications
/// were arriving and being acked; see the note in `notifications.rs`.
#[cfg(feature = "storage-rocksdb")]
async fn record_delivery(state: &AppState, tenant: &str, repo: &str, mount_id: &str) {
    let Some(storage) = state.rocksdb_storage() else {
        return;
    };
    if let Err(e) = raisin_rocksdb::record_push_delivery(
        storage,
        tenant,
        repo,
        super::CONFIG_BRANCH,
        mount_id,
        true,
    )
    .await
    {
        tracing::warn!(mount_id = %mount_id, error = %e, "failed to record push delivery health");
    }
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn connect_event(
    State(_): State<AppState>,
    Extension(_): Extension<TenantInfo>,
    Json(_): Json<ConnectEventRequest>,
) -> Result<Response, ApiError> {
    Err(ApiError::internal("RocksDB backend required"))
}

#[cfg(test)]
mod tests {
    use super::wants_push;
    use serde_json::{json, Value};

    fn accounts() -> Vec<String> {
        vec!["acc-1".to_string(), "acc-2".to_string()]
    }

    fn check(sync: Value, account_ref: Value) -> bool {
        wants_push(&Value::Null, &Value::Null, &sync, &account_ref, &accounts())
    }

    #[test]
    fn a_hybrid_mount_on_a_matched_account_wants_push() {
        assert!(check(json!({ "mode": "hybrid" }), json!("acc-1")));
        assert!(check(json!({ "mode": "webhook" }), json!("acc-2")));
    }

    /// `poll` is the operator saying "interval only". Push must not override it, and an
    /// absent mode is poll — the engine's own default.
    #[test]
    fn a_poll_mount_is_left_alone() {
        assert!(!check(json!({ "mode": "poll" }), json!("acc-1")));
        assert!(!check(json!({}), json!("acc-1")));
        assert!(!check(Value::Null, json!("acc-1")));
    }

    #[test]
    fn disabled_and_paused_mounts_are_skipped() {
        let sync = json!({ "mode": "hybrid" });
        assert!(!wants_push(
            &json!(false),
            &Value::Null,
            &sync,
            &json!("acc-1"),
            &accounts()
        ));
        assert!(!wants_push(
            &Value::Null,
            &json!({ "paused": true }),
            &sync,
            &json!("acc-1"),
            &accounts()
        ));
    }

    /// The cross-account guard: a mount pinned to a DIFFERENT connection must not be woken
    /// by this account's events, or one Stripe account's activity silently resyncs
    /// another's data.
    #[test]
    fn a_mount_pinned_to_another_account_is_not_matched() {
        assert!(!check(json!({ "mode": "webhook" }), json!("acc-other")));
    }

    /// An absent `account_ref` means "this repo's connection", and the caller has already
    /// established the account is connected here.
    #[test]
    fn an_absent_account_ref_matches() {
        assert!(check(json!({ "mode": "hybrid" }), Value::Null));
    }
}
