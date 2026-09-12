// SPDX-License-Identifier: BSL-1.1

//! `POST /api/integrations/{repo}/mounts/{mount_id}/rebind` (one mount) and
//! `POST /api/integrations/{repo}/mounts/rebind` (a group of them) — point
//! mounts at a different connection on their connector.
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
//!
//! # Why there is a BATCH form
//!
//! Disconnecting one connection orphans every mount pinned to it, and in
//! practice that is never one mount: a Microsoft 365 connector carries an inbox,
//! a sent folder, drafts, an outbox and two calendars, all pinned to the SAME
//! account id. Six identical repairs is not six decisions — it is one decision
//! typed six times, and an operator halfway through has a connector whose mounts
//! disagree about which mailbox they sync.
//!
//! So the batch form takes ONE `account_id` and a list of mounts, and it is
//! deliberately keyed that way rather than as "repair everything broken": mounts
//! that pointed at DIFFERENT removed connections were different mailboxes, and
//! collapsing them into one choice is exactly the wrong-mailbox failure this
//! endpoint exists to avoid. Grouping by the dangling id is the caller's job and
//! the console does it; the server just refuses anything that does not resolve.
//!

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

/// Body for the batch rebind.
#[derive(Debug, Deserialize)]
pub struct RebindMountsRequest {
    /// The connection every listed mount should bind to.
    pub account_id: String,
    /// The mounts to repair. The caller groups these — see the module docs.
    pub mount_ids: Vec<String>,
}

/// Upper bound on one batch, so a malformed caller cannot walk every node in
/// the repo inside one request.
const MAX_BATCH: usize = 100;

/// What happened to one mount in a batch.
#[derive(Debug, Serialize)]
pub struct RebindMountOutcome {
    pub mount_id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Why this one failed. Present exactly when `ok` is false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Result of a batch rebind.
#[derive(Debug, Serialize)]
pub struct RebindMountsResponse {
    /// Every mount in the batch succeeded. A batch that repaired five of six
    /// still returns 200 — read `results`.
    pub ok: bool,
    pub account_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub rebound: usize,
    pub failed: usize,
    pub results: Vec<RebindMountOutcome>,
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
    let outcome = rebind_one(&svc, &mount_id, &account_id).await?;

    Ok(Json(RebindMountResponse {
        ok: true,
        account_id,
        previous_account_id: outcome.previous_account_id,
        label: outcome.label,
    }))
}

/// `POST /api/integrations/{repo}/mounts/rebind`
///
/// One connection, many mounts. Every mount is validated against ITS OWN
/// connector, so a list that accidentally spans two connectors fails per mount
/// rather than binding some of them to a connection that is not theirs.
///
/// # Partial success is reported, not hidden
///
/// The mounts are repaired one at a time and the response carries a per-mount
/// verdict. There is deliberately no transaction: each mount is an independent
/// node write, and the alternative — refusing the whole batch because one mount
/// was deleted while the console page was open — would leave five repairable
/// mounts broken to punish the sixth. `ok` is therefore "every mount in this
/// batch succeeded", and the caller must read `results` rather than trusting a
/// 200.
pub async fn rebind_mounts(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<RebindMountsRequest>,
) -> Result<Json<RebindMountsResponse>, ApiError> {
    require_admin(auth.as_deref())?;

    let account_id = req.account_id.trim().to_string();
    if account_id.is_empty() {
        return Err(ApiError::validation_failed("account_id is required"));
    }
    if req.mount_ids.is_empty() {
        return Err(ApiError::validation_failed("mount_ids is required"));
    }
    if req.mount_ids.len() > MAX_BATCH {
        return Err(ApiError::validation_failed(format!(
            "at most {MAX_BATCH} mounts can be rebound in one call, got {}",
            req.mount_ids.len()
        )));
    }

    let svc = super::config_service(&state, &tenant.tenant_id, &repo, ACTOR);

    let mut results = Vec::with_capacity(req.mount_ids.len());
    let mut label = None;
    let mut rebound = 0usize;
    // De-duplicate: the console builds this list from a rendered table, and a
    // repeated id would otherwise be reported twice with the second look
    // showing the FIRST repair as its "previous" value — which reads as if the
    // mount had been bound to the new connection all along.
    let mut seen = std::collections::HashSet::new();
    for mount_id in req.mount_ids.iter().filter(|id| seen.insert(id.as_str())) {
        match rebind_one(&svc, mount_id, &account_id).await {
            Ok(outcome) => {
                rebound += 1;
                label = label.or(outcome.label.clone());
                results.push(RebindMountOutcome {
                    mount_id: mount_id.clone(),
                    ok: true,
                    previous_account_id: outcome.previous_account_id,
                    label: outcome.label,
                    error: None,
                });
            }
            Err(e) => results.push(RebindMountOutcome {
                mount_id: mount_id.clone(),
                ok: false,
                previous_account_id: None,
                label: None,
                error: Some(e.into_message()),
            }),
        }
    }

    let failed = results.len() - rebound;
    tracing::info!(
        repo = %repo,
        to = %account_id,
        rebound,
        failed,
        "rebound a group of mounts to one connection"
    );

    Ok(Json(RebindMountsResponse {
        ok: failed == 0,
        account_id,
        label,
        rebound,
        failed,
        results,
    }))
}

/// What one successful rebind produced, for whichever response shape wraps it.
struct RebindOutcome {
    previous_account_id: Option<String>,
    label: Option<String>,
}

/// Rebind ONE mount, validating the target against that mount's own connector.
///
/// The single and batch routes share this body rather than each doing their own
/// lookups: the checks below are the whole safety of the operation, and a batch
/// path that reimplemented them loosely — skipping the node-type guard, or
/// validating against the first mount's connector — would be a bulk version of
/// exactly the wrong-mailbox failure the endpoint exists to prevent.
async fn rebind_one(
    svc: &raisin_core::NodeService<crate::state::Store>,
    mount_id: &str,
    account_id: &str,
) -> Result<RebindOutcome, ApiError> {
    let mut node = svc
        .get(mount_id)
        .await?
        .ok_or_else(|| ApiError::node_not_found(mount_id.to_string()))?;

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

    let connector = load_connector(svc, &integration_ref)
        .await?
        .ok_or_else(|| ApiError::node_not_found(integration_ref.clone()))?;

    let connections = super::connected_accounts(&connector);
    let target = connections
        .iter()
        .find(|a| a.get("id").and_then(|v| v.as_str()) == Some(account_id))
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

    // Already bound: report success without a write. A group repair re-run
    // after a partial failure would otherwise mint a node revision — replicated
    // — for every mount that was already fine.
    if previous.as_deref() == Some(account_id) {
        return Ok(RebindOutcome {
            previous_account_id: previous,
            label,
        });
    }

    node.properties.insert(
        "account_ref".to_string(),
        raisin_models::nodes::properties::PropertyValue::String(account_id.to_string()),
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

    Ok(RebindOutcome {
        previous_account_id: previous,
        label,
    })
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
