// SPDX-License-Identifier: BSL-1.1

//! `DELETE /api/integrations/{repo}/mounts/{mount_id}` — delete a virtual mount
//! *and* unregister its push subscription with the provider.
//!
//! This exists because deleting the mount node on its own leaks a live provider
//! subscription. The console used to delete a mount with a generic node delete,
//! which the engine never sees: the sync job's teardown path
//! (`virtual_mount_sync::teardown_push`) only runs for a mount that still EXISTS
//! and is merely `enabled: false`, and the job bails early once the node is gone.
//! The provider therefore keeps POSTing notifications at a URL that no longer
//! resolves — measured in production as 1,528 Microsoft Graph notifications
//! 404'ing in 24 hours, for two mounts deleted long before, with no backoff.
//!
//! Order matters: unsubscribe FIRST, then delete. After the node is gone the
//! `push_subscription_id` is unrecoverable, so a delete-then-unsubscribe would
//! leak the subscription whenever the provider call fails.
//!
//! The unsubscribe is **best-effort**. A provider that is down, an expired
//! credential, or a connector whose adapter was removed must not make a mount
//! undeletable — the operator would be left with a node they cannot remove and
//! the leak they were trying to fix. Failure is logged at `warn!` and reported
//! as `unsubscribed: false`.
//!
//! The adapter is invoked through the shared [`super::adapter_invoke`] path used
//! by `test`/`browse`, with the credential and mount snapshot built by the same
//! helpers, so this endpoint cannot drift from what a real sync would do.

#![cfg(feature = "storage-rocksdb")]

use std::time::Duration;

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::Node;
use serde::Serialize;
use serde_json::{json, Value};

use super::adapter_invoke::{invoke_adapter, load_adapter_node, AdapterResult};
use super::require_admin;
use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::AppState;

/// Node type this endpoint is allowed to delete. Checked so the route cannot be
/// used as a generic delete for anything else in `raisin:system`.
const MOUNT_NODE_TYPE: &str = "raisin:VirtualMount";

/// Wall-clock ceiling on the provider unsubscribe. A deletion runs while someone
/// waits, and an unreachable provider must not hold the request open — the
/// subscription expires on the provider side anyway.
const UNSUBSCRIBE_TIMEOUT: Duration = Duration::from_secs(15);

/// Actor recorded on the config reads/writes this endpoint performs.
const ACTOR: &str = "integration-mount-delete";

/// Response for the mount delete.
#[derive(Debug, Serialize)]
pub struct DeleteMountResponse {
    /// Always `true` on a 2xx — the mount node was removed.
    pub deleted: bool,
    /// Whether the provider subscription was successfully unregistered. `false`
    /// also covers "there was no subscription to unregister", which is the
    /// normal case for a poll-mode mount.
    pub unsubscribed: bool,
}

/// Delete a virtual mount, unsubscribing from its provider first.
pub async fn delete_mount(
    State(state): State<AppState>,
    Path((repo, mount_id)): Path<(String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<DeleteMountResponse>, ApiError> {
    require_admin(auth.as_deref())?;

    let svc = super::config_service(&state, &tenant.tenant_id, &repo, ACTOR);
    let node = svc
        .get(&mount_id)
        .await?
        .filter(|n| n.node_type == MOUNT_NODE_TYPE)
        .ok_or_else(|| ApiError::node_not_found(mount_id.clone()))?;

    // Unsubscribe BEFORE the delete: once the node is gone the subscription id
    // cannot be recovered.
    let unsubscribed = match push_subscription_id(&node) {
        None => false,
        Some(sub) => unsubscribe(&state, &tenant.tenant_id, &repo, &node, &sub).await,
    };

    let deleted = svc.delete(&mount_id).await?;
    if !deleted {
        return Err(ApiError::internal(format!(
            "virtual mount `{mount_id}` could not be deleted"
        )));
    }

    tracing::info!(
        mount_id = %mount_id,
        unsubscribed,
        "virtual mount deleted"
    );

    Ok(Json(DeleteMountResponse {
        deleted,
        unsubscribed,
    }))
}

/// Best-effort provider unsubscribe. Returns whether the adapter reported
/// success; every failure path logs a `warn!` and returns `false` rather than
/// propagating, so the delete always proceeds.
async fn unsubscribe(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    mount: &Node,
    subscription_id: &str,
) -> bool {
    let Some(integration_ref) = string_prop(mount, "integration_ref") else {
        tracing::warn!(
            mount_id = %mount.id,
            "cannot unsubscribe: mount has no integration_ref; the provider subscription is leaked"
        );
        return false;
    };

    // `integration_ref` may be a path or a node id — the engine tries both, so
    // this must too or the two disagree about which connector a mount points at.
    let svc = super::config_service(state, tenant_id, repo, ACTOR);
    let integ_node = match svc.get_by_path(&integration_ref).await {
        Ok(Some(n)) => Some(n),
        _ => svc.get(&integration_ref).await.ok().flatten(),
    };
    let Some(integ_node) = integ_node else {
        tracing::warn!(
            mount_id = %mount.id,
            integration_ref = %integration_ref,
            "cannot unsubscribe: connector node not found; the provider subscription is leaked"
        );
        return false;
    };

    let Some(adapter_path) = adapter_path(mount, &integ_node) else {
        tracing::warn!(
            mount_id = %mount.id,
            "cannot unsubscribe: no adapter_function on the mount or its connector; \
             the provider subscription is leaked"
        );
        return false;
    };

    let adapter_node = match load_adapter_node(state, tenant_id, repo, &adapter_path, ACTOR).await {
        Ok(Some(n)) => n,
        _ => {
            tracing::warn!(
                mount_id = %mount.id,
                adapter = %adapter_path,
                "cannot unsubscribe: adapter function not found; the provider subscription is leaked"
            );
            return false;
        }
    };

    // The connection whose credential the subscription was registered with.
    // Resolved by the shared truth table, so an ambiguous connector fails here
    // exactly as it would in a sync rather than guessing a mailbox.
    let provider_type = string_prop(&integ_node, "provider_type").unwrap_or_default();
    let account_id = match account_id_for(mount, &integ_node) {
        Ok(id) => id,
        Err(reason) => {
            tracing::warn!(
                mount_id = %mount.id,
                reason = %reason,
                "cannot unsubscribe: no usable connection; the provider subscription is leaked"
            );
            return false;
        }
    };
    let credential =
        super::test_connection::resolve_credential(state, &integ_node, &provider_type, &account_id);
    if credential.is_none() {
        tracing::warn!(
            mount_id = %mount.id,
            "cannot unsubscribe: no decryptable token for the mount's connection; \
             the provider subscription is leaked"
        );
        return false;
    }

    let snapshot = unsubscribe_snapshot(mount, &integ_node, &account_id);
    let result = tokio::time::timeout(
        UNSUBSCRIBE_TIMEOUT,
        invoke_adapter(
            state,
            tenant_id,
            repo,
            &adapter_node,
            "unsubscribe",
            json!({ "subscription_id": subscription_id }),
            &credential,
            &snapshot,
        ),
    )
    .await;

    match result {
        Ok(Ok(AdapterResult::Ok(_))) => true,
        Ok(Ok(AdapterResult::Failed(msg))) => {
            tracing::warn!(
                mount_id = %mount.id,
                error = %super::test_connection::sanitize(&msg),
                "adapter unsubscribe failed; deleting the mount anyway — the provider \
                 subscription may keep sending notifications until it expires"
            );
            false
        }
        Ok(Err(e)) => {
            tracing::warn!(
                mount_id = %mount.id,
                error = %e,
                "adapter unsubscribe could not be invoked; deleting the mount anyway"
            );
            false
        }
        Err(_) => {
            tracing::warn!(
                mount_id = %mount.id,
                "adapter unsubscribe timed out after {}s; deleting the mount anyway",
                UNSUBSCRIBE_TIMEOUT.as_secs()
            );
            false
        }
    }
}

// --- Pure helpers (unit-tested below) ---

/// The mount's live provider subscription id, or `None` when it has none (a
/// poll-mode mount, or one whose subscription was already torn down).
fn push_subscription_id(mount: &Node) -> Option<String> {
    super::json_prop(mount, "state")
        .get("push_subscription_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// The adapter to invoke: the mount's own override, else its connector's.
/// Mirrors the engine's resolution order in `build_ctx_parts`.
fn adapter_path(mount: &Node, integration: &Node) -> Option<String> {
    string_prop(mount, "adapter_function").or_else(|| string_prop(integration, "adapter_function"))
}

/// Which connection the mount authenticates as, via the shared selection rules
/// (explicit `account_ref` must match; with none, exactly one connection
/// resolves). Returns the selection error's message on failure.
fn account_id_for(mount: &Node, integration: &Node) -> Result<String, String> {
    use raisin_models::nodes::integrations::{AccountSelection, IntegrationConfig};
    let config = IntegrationConfig::from_node(integration)?;
    let account_ref = string_prop(mount, "account_ref");
    AccountSelection::resolve(&config.accounts, account_ref.as_deref())
        .map(|a| a.id.clone())
        .map_err(|e| e.to_string())
}

/// The `mount` object handed to the adapter for the unsubscribe call.
///
/// Built by the same [`super::test_connection::mount_snapshot`] the connection
/// test and browse endpoints use — so the adapter sees the identical merged
/// `config` (connector- and connection-level settings: the Entra tenant, the
/// shared mailbox principal, the API host) — with the mount's REAL identity and
/// `sync_config` layered on top. Those three fields are what distinguishes this
/// from a probe: an adapter that keys anything off the mount would otherwise be
/// told it is tearing down `connection-test`.
fn unsubscribe_snapshot(mount: &Node, integration: &Node, account_id: &str) -> Value {
    let remote_root = string_prop(mount, "remote_root");
    let sync_config = super::json_prop(mount, "sync_config");
    let mut snapshot = super::test_connection::mount_snapshot(
        &remote_root,
        integration,
        Some(account_id),
        Some(&sync_config),
    );
    if let Some(obj) = snapshot.as_object_mut() {
        obj.insert("mount_id".into(), Value::String(mount.id.clone()));
        if let Some(path) = string_prop(mount, "mount_path") {
            obj.insert("mount_path".into(), Value::String(path));
        }
    }
    snapshot
}

/// Read a non-empty string property.
fn string_prop(node: &Node, key: &str) -> Option<String> {
    use raisin_models::nodes::properties::PropertyValue;
    match node.properties.get(key)? {
        PropertyValue::String(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mount(props: Value) -> Node {
        serde_json::from_value(json!({
            "id": "mount-1",
            "name": "mail",
            "node_type": MOUNT_NODE_TYPE,
            "properties": props,
        }))
        .expect("mount node")
    }

    fn connector(props: Value) -> Node {
        serde_json::from_value(json!({
            "id": "integ-1",
            "name": "ms-graph",
            "node_type": "raisin:Integration",
            "properties": props,
        }))
        .expect("connector node")
    }

    /// The case the endpoint must never get wrong: a poll-mode mount has nothing
    /// to unsubscribe, so it deletes cleanly and reports `unsubscribed: false`.
    /// No adapter is invoked at all — nothing here reaches a provider.
    #[test]
    fn mount_without_subscription_has_nothing_to_unsubscribe() {
        assert!(push_subscription_id(&mount(json!({ "mount_path": "/mail" }))).is_none());
        // An engine-written state blob that simply never subscribed.
        assert!(push_subscription_id(&mount(json!({
            "state": { "status": "ok", "last_sync_at": 1_700_000_000 }
        })))
        .is_none());
        // A torn-down subscription serializes the field away, but an explicit
        // empty string must be treated the same.
        assert!(push_subscription_id(&mount(json!({
            "state": { "push_subscription_id": "" }
        })))
        .is_none());

        // The response this produces.
        let resp = DeleteMountResponse {
            deleted: true,
            unsubscribed: false,
        };
        assert_eq!(
            serde_json::to_value(&resp).unwrap(),
            json!({ "deleted": true, "unsubscribed": false })
        );
    }

    #[test]
    fn subscription_id_is_read_from_engine_state() {
        let node = mount(json!({
            "state": {
                "push_subscription_id": "9e93b593-1dca-4092-8ece-f6941dc94759",
                "push_status": "active"
            }
        }));
        assert_eq!(
            push_subscription_id(&node).as_deref(),
            Some("9e93b593-1dca-4092-8ece-f6941dc94759")
        );
    }

    #[test]
    fn adapter_path_prefers_the_mount_override() {
        let integ = connector(json!({ "adapter_function": "/adapters/ms-graph" }));
        assert_eq!(
            adapter_path(&mount(json!({})), &integ).as_deref(),
            Some("/adapters/ms-graph")
        );
        assert_eq!(
            adapter_path(
                &mount(json!({ "adapter_function": "/adapters/custom" })),
                &integ
            )
            .as_deref(),
            Some("/adapters/custom")
        );
        assert!(adapter_path(&mount(json!({})), &connector(json!({}))).is_none());
    }

    #[test]
    fn account_selection_follows_the_shared_rules() {
        let one = connector(json!({
            "provider_type": "ms-graph",
            "connected_accounts": [{ "id": "acct-1" }]
        }));
        // No account_ref + exactly one connection resolves.
        assert_eq!(account_id_for(&mount(json!({})), &one).unwrap(), "acct-1");
        // An explicit ref wins.
        let two = connector(json!({
            "provider_type": "ms-graph",
            "connected_accounts": [{ "id": "acct-1" }, { "id": "acct-2" }]
        }));
        assert_eq!(
            account_id_for(&mount(json!({ "account_ref": "acct-2" })), &two).unwrap(),
            "acct-2"
        );
        // Ambiguous and missing are errors, not a guess — the caller logs and
        // skips the unsubscribe rather than tearing down some other mailbox's
        // subscription.
        assert!(account_id_for(&mount(json!({})), &two).is_err());
        assert!(account_id_for(&mount(json!({})), &connector(json!({}))).is_err());
        assert!(account_id_for(&mount(json!({ "account_ref": "gone" })), &one).is_err());
    }

    /// The adapter must be told which mount it is tearing down, not the probe's
    /// placeholder id, while still receiving the same merged connector +
    /// connection config a sync would.
    #[test]
    fn snapshot_carries_the_real_mount_identity_and_merged_config() {
        let m = mount(json!({
            "mount_path": "/mail/inbox",
            "remote_root": "AAMkAG",
            "sync_config": { "mode": "webhook", "resource": "mail" }
        }));
        let integ = connector(json!({
            "provider_type": "ms-graph",
            "config": { "default_tenant_id": "tenant-guid" },
            "connected_accounts": [{
                "id": "acct-1",
                "config": { "principal": "sales@contoso.com" }
            }]
        }));
        let snap = unsubscribe_snapshot(&m, &integ, "acct-1");

        assert_eq!(snap["mount_id"], "mount-1");
        assert_eq!(snap["mount_path"], "/mail/inbox");
        assert_eq!(snap["remote_root"], "AAMkAG");
        // Connector + connection layers reach the adapter.
        assert_eq!(snap["config"]["default_tenant_id"], "tenant-guid");
        assert_eq!(snap["config"]["principal"], "sales@contoso.com");
        // The mount's own surface selector survives.
        assert_eq!(snap["sync_config"]["resource"], "mail");
    }
}
