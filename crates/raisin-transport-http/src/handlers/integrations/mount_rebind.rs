// SPDX-License-Identifier: BSL-1.1

//! `POST /api/integrations/{repo}/mounts/{mount_id}/rebind` — point a mount at
//! a different connection on its connector.
//!
//! # Why this exists
//!
//! `account_ref` is engine-owned and deliberately NOT writable through
//! `patch_sync_config` (`mount_config::fields`), which is right: it decides
//! WHICH MAILBOX a mount syncs, and a generic config patch is the wrong shape
//! for a change with that blast radius.
//!
//! The consequence, until now, was that a mount whose connection disappeared had
//! **no repair path at all**. That is not a hypothetical state:
//!
//! ```text
//! mount does not resolve to a single connection; marking misconfigured
//! error=connected account `vvODAeBjosO6IFhsRCz0k` no longer exists on this connector
//! ```
//!
//! …once a minute, indefinitely, on a live tenant. Disconnecting a connection
//! orphans every mount pinned to it, and re-connecting mints a NEW account id
//! rather than reviving the old one — so the operator consents, is told it
//! worked, and the mounts go on failing against an id that will never come back.
//! The only remedy was deleting and recreating the mount, which discards its
//! sync state and re-imports everything.
//!
//! # Why it is explicit
//!
//! Rebinding is a deliberate, named action rather than something the engine does
//! for you. A mount whose `account_ref` has gone missing COULD be silently
//! re-pointed at the connector's remaining connection, and on a connector with
//! exactly one that would even be right most of the time — but "most of the
//! time" is doing far too much work. The failure mode is materialising one
//! person's mailbox under a path that was syncing another's, with no record that
//! the subject changed. Failing loudly and making the repair one call is the
//! trade this takes.

#![cfg(feature = "storage-rocksdb")]

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use raisin_models::auth::AuthContext;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::require_admin;
use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::AppState;

/// Actor recorded on the config write.
const ACTOR: &str = "integration-mount-rebind";
/// NodeType guard, so this route cannot be aimed at an arbitrary node.
const MOUNT_NODE_TYPE: &str = "raisin:VirtualMount";

/// Body for the rebind.
#[derive(Debug, Deserialize)]
pub struct RebindMountRequest {
    /// The connection to bind to. Must already exist on the mount's connector.
    pub account_id: String,
}

/// Result of a rebind.
#[derive(Debug, Serialize)]
pub struct RebindMountResponse {
    pub ok: bool,
    /// The connection the mount now uses.
    pub account_id: String,
    /// What it was bound to before, if anything — worth echoing so an operator
    /// repairing several mounts can see which were actually dangling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_account_id: Option<String>,
    /// The connection's human label, for confirming the right mailbox was
    /// chosen before anything syncs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// `POST /api/integrations/{repo}/mounts/{mount_id}/rebind`
pub async fn rebind_mount(
    State(state): State<AppState>,
    Path((repo, mount_id)): Path<(String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<RebindMountRequest>,
) -> Result<Json<RebindMountResponse>, ApiError> {
    require_admin(auth.as_deref())?;

    let account_id = req.account_id.trim().to_string();
    if account_id.is_empty() {
        return Err(ApiError::validation_failed("account_id is required"));
    }

    let svc = super::config_service(&state, &tenant.tenant_id, &repo, ACTOR);
    let mut node = svc
        .get(&mount_id)
        .await?
        .ok_or_else(|| ApiError::node_not_found(mount_id.clone()))?;

    if node.node_type != MOUNT_NODE_TYPE {
        return Err(ApiError::validation_failed(format!(
            "node `{mount_id}` is a {}, not a {MOUNT_NODE_TYPE}",
            node.node_type
        )));
    }

    // Resolve the mount's connector, so the target connection is validated
    // against the SAME connector the mount syncs through. Without this check a
    // typo would replace one dangling reference with another and the mount would
    // keep failing with a different id in the message.
    let integration_ref = super::json_prop(&node, "integration_ref")
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| {
            ApiError::validation_failed(format!("mount `{mount_id}` has no integration_ref"))
        })?;

    let connector = load_connector(&svc, &integration_ref)
        .await?
        .ok_or_else(|| ApiError::node_not_found(integration_ref.clone()))?;

    let connections = super::connected_accounts(&connector);
    let target = connections
        .iter()
        .find(|a| a.get("id").and_then(|v| v.as_str()) == Some(account_id.as_str()))
        .ok_or_else(|| {
            // Name what IS available. An operator repairing a dangling reference
            // has, by definition, an id that does not exist; telling them only
            // that is unhelpful when the whole point is to pick a live one.
            let available: Vec<String> = connections
                .iter()
                .filter_map(|a| {
                    let id = a.get("id").and_then(|v| v.as_str())?;
                    let label = a
                        .get("label")
                        .or_else(|| a.get("subject"))
                        .and_then(|v| v.as_str())
                        .unwrap_or(id);
                    Some(format!("{id} ({label})"))
                })
                .collect();
            ApiError::validation_failed(if available.is_empty() {
                format!(
                    "connector `{integration_ref}` has no connections at all — connect an \
                     account before rebinding this mount"
                )
            } else {
                format!(
                    "connector `{integration_ref}` has no connection `{account_id}`; \
                     available: {}",
                    available.join(", ")
                )
            })
        })?;

    let label = target
        .get("label")
        .or_else(|| target.get("subject"))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let previous = super::json_prop(&node, "account_ref")
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    node.properties.insert(
        "account_ref".to_string(),
        raisin_models::nodes::properties::PropertyValue::String(account_id.clone()),
    );

    // The mount keeps its `state`, and that is the point of repairing rather
    // than recreating: its cursors, its backfill progress and its item history
    // all survive. The next sync re-runs preflight, which clears the
    // `misconfigured` status on its own once the reference resolves.
    svc.update_node(node).await?;

    tracing::info!(
        mount_id = %mount_id,
        integration = %integration_ref,
        from = previous.as_deref().unwrap_or("<unset>"),
        to = %account_id,
        "mount rebound to a different connection"
    );

    Ok(Json(RebindMountResponse {
        ok: true,
        account_id,
        previous_account_id: previous,
        label,
    }))
}

/// Load the connector a mount references, by node id first and then by path.
///
/// `integration_ref` carries either spelling depending on how the mount was
/// created, and a repair endpoint that understood only one of them would refuse
/// exactly the older mounts most likely to need repairing.
async fn load_connector(
    svc: &raisin_core::NodeService<crate::state::Store>,
    integration_ref: &str,
) -> Result<Option<raisin_models::nodes::Node>, ApiError> {
    if let Some(node) = svc.get(integration_ref).await? {
        return Ok(Some(node));
    }
    Ok(svc.get_by_path(integration_ref).await?)
}

/// Whether a mount's `account_ref` names a connection that no longer exists.
///
/// Shared with the mount listing so the console can offer the repair exactly
/// where the failure shows, rather than making an operator correlate a log line
/// with a mount id.
pub(crate) fn dangling_account_ref(mount: &Value, connections: &[Value]) -> Option<String> {
    let account_ref = mount
        .get("account_ref")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())?;
    let exists = connections
        .iter()
        .any(|a| a.get("id").and_then(|v| v.as_str()) == Some(account_ref));
    (!exists).then(|| account_ref.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn connections() -> Vec<Value> {
        vec![
            json!({ "id": "live-1", "label": "support@example.test" }),
            json!({ "id": "live-2", "label": "billing@example.test" }),
        ]
    }

    #[test]
    fn a_reference_to_a_removed_connection_is_dangling() {
        let mount = json!({ "account_ref": "vvODAeBjosO6IFhsRCz0k" });
        assert_eq!(
            dangling_account_ref(&mount, &connections()).as_deref(),
            Some("vvODAeBjosO6IFhsRCz0k")
        );
    }

    #[test]
    fn a_live_reference_is_not_dangling() {
        let mount = json!({ "account_ref": "live-2" });
        assert!(dangling_account_ref(&mount, &connections()).is_none());
    }

    /// An UNSET `account_ref` is not dangling — it means "this connector has one
    /// connection, use it", which is a supported configuration. Reporting it as
    /// broken would flag most mounts in every deployment.
    #[test]
    fn an_unset_reference_is_not_dangling() {
        assert!(dangling_account_ref(&json!({}), &connections()).is_none());
        assert!(dangling_account_ref(&json!({ "account_ref": "" }), &connections()).is_none());
    }

    /// A connector with no connections at all: an explicit reference is still
    /// dangling, because the mount names something that is not there.
    #[test]
    fn a_reference_on_a_connectorless_connector_is_dangling() {
        let mount = json!({ "account_ref": "gone" });
        assert_eq!(dangling_account_ref(&mount, &[]).as_deref(), Some("gone"));
    }
}
