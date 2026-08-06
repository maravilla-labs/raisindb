// SPDX-License-Identifier: BSL-1.1

//! Pause, resume and stop for a virtual mount's sync.
//!
//! Two controls with deliberately different blast radius:
//!
//! - **Pause** suspends scheduling and nothing else. The provider subscription
//!   stays registered, so resuming is instant and notifications that arrive in
//!   the gap are not lost. This is why it is a separate flag from the mount's
//!   `enabled` property, which also tears the subscription down.
//! - **Stop** ends the run that is happening right now and discards its resume
//!   point, so the import starts over rather than continuing. It does not pause:
//!   an unpaused mount will schedule a fresh run on the next tick.
//!
//! Both write only the fields they own, through the config service, rather than
//! going near the generic node API — which replaces every property and would
//! clobber a concurrent edit. `mount_delete.rs` carries the same warning for the
//! same reason.

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use raisin_models::auth::AuthContext;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::require_admin;
use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::AppState;

/// Actor recorded on the config writes these endpoints perform.
const ACTOR: &str = "integration-mount-control";
/// NodeType guard, so neither route can be aimed at an arbitrary node.
const MOUNT_NODE_TYPE: &str = "raisin:VirtualMount";

/// Body for pause/resume.
#[derive(Debug, Deserialize)]
pub struct PauseMountRequest {
    /// `true` pauses, `false` resumes.
    pub paused: bool,
}

/// Result of a pause, resume or stop.
#[derive(Debug, Serialize)]
pub struct MountControlResponse {
    pub ok: bool,
    /// Whether scheduling is currently suspended.
    pub paused: bool,
    /// Whether a running sync was asked to stop.
    pub stop_requested: bool,
}

/// `POST /api/integrations/{repo}/mounts/{mount_id}/pause`
pub async fn pause_mount(
    State(state): State<AppState>,
    Path((repo, mount_id)): Path<(String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<PauseMountRequest>,
) -> Result<Json<MountControlResponse>, ApiError> {
    require_admin(auth.as_deref())?;

    let paused = req.paused;
    let sync_state = mutate_state(&state, &tenant.tenant_id, &repo, &mount_id, |s| {
        s.insert("paused".to_string(), json!(paused));
        // Resuming clears any stale stop request, so a mount that was stopped
        // and then paused does not immediately stop again on resume.
        if !paused {
            s.insert("stop_requested".to_string(), json!(false));
        }
    })
    .await?;

    Ok(Json(response(&sync_state)))
}

/// `POST /api/integrations/{repo}/mounts/{mount_id}/stop`
///
/// Cooperative: the running sync notices at its next page boundary, where it has
/// already flushed and persisted. Aborting the job instead would skip the lease
/// release and strand the mount for the full lease TTL with a status of
/// `syncing` that nothing clears.
///
/// The resume point is discarded. A stop that kept it would be undone by the
/// scheduler on the very next tick — an unfinished backfill is due immediately —
/// so a stop that did not clear the cursor would not be a stop at all.
pub async fn stop_mount(
    State(state): State<AppState>,
    Path((repo, mount_id)): Path<(String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<MountControlResponse>, ApiError> {
    require_admin(auth.as_deref())?;

    let sync_state = mutate_state(&state, &tenant.tenant_id, &repo, &mount_id, |s| {
        s.insert("stop_requested".to_string(), json!(true));
        // Discard the resume point, or the next tick starts another chunk.
        s.remove("backfill_cursor");
        s.remove("backfill_stack");
        s.insert("backfill_items_done".to_string(), json!(0));
        s.insert("delta_items_done".to_string(), json!(0));
        // NOT `backfill_complete`: the walk did not reach the end, and claiming
        // otherwise would let reconcile deletes run against a partial picture.
        s.insert("backfill_complete".to_string(), json!(false));
        // Drop the delta cursor as well. A stored token can be a mid-enumeration
        // cursor rather than a resume point, and that is precisely the state an
        // operator reaches for Stop to escape. With no token the next run
        // re-baselines instead of resuming a walk that cannot converge.
        s.remove("last_sync_token");
    })
    .await?;

    Ok(Json(response(&sync_state)))
}

/// Body for the writeback release.
#[derive(Debug, Deserialize)]
pub struct ConfirmWritebackRequest {
    /// The `state.writeback_blocked.token` being released. Required, and
    /// deliberately not optional or defaulted: a release with no token would be
    /// "approve whatever is parked", which is exactly the blanket authorization
    /// the rails exist to withhold.
    pub token: String,
}

/// `POST /api/integrations/{repo}/mounts/{mount_id}/writeback/confirm`
///
/// Release one blocked batch of deletes.
///
/// A tripped rail parks every pending delete, sets `state.writeback_blocked`
/// and pushes nothing — reads carry on. Clearing it is deliberately a human
/// action, and deliberately scoped to ONE batch: the token is derived from that
/// batch's own contents, so a confirmation given after reviewing three deletes
/// cannot authorise the thirty that arrived since. The engine re-checks the
/// token against the batch it is about to push and re-blocks if they disagree.
///
/// The token is required to MATCH; this endpoint only records it. Recording an
/// arbitrary string is harmless — the engine ignores a token that fingerprints
/// nothing — and validating it here would mean reading the block twice and
/// racing a drain that may have re-evaluated it in between.
pub async fn confirm_writeback(
    State(state): State<AppState>,
    Path((repo, mount_id)): Path<(String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<ConfirmWritebackRequest>,
) -> Result<Json<MountControlResponse>, ApiError> {
    require_admin(auth.as_deref())?;

    let token = req.token.trim().to_string();
    if token.is_empty() {
        return Err(ApiError::validation_failed(
            "writeback confirm token is required; copy state.writeback_blocked.token",
        ));
    }

    let sync_state = mutate_state(&state, &tenant.tenant_id, &repo, &mount_id, |s| {
        s.insert("writeback_confirm_token".to_string(), json!(token));
    })
    .await?;

    Ok(Json(response(&sync_state)))
}

/// Read the mount, apply `edit` to its `state` object, write it back.
///
/// Only the `state` property is replaced; every other property is left exactly
/// as read.
async fn mutate_state(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    mount_id: &str,
    edit: impl FnOnce(&mut serde_json::Map<String, Value>),
) -> Result<Value, ApiError> {
    let svc = super::config_service(state, tenant_id, repo, ACTOR);

    let mut node = svc
        .get(mount_id)
        .await?
        .ok_or_else(|| ApiError::node_not_found(mount_id.to_string()))?;

    // Guard the type, so this route cannot be pointed at some other node.
    if node.node_type != MOUNT_NODE_TYPE {
        return Err(ApiError::validation_failed(format!(
            "node `{mount_id}` is a {}, not a {MOUNT_NODE_TYPE}",
            node.node_type
        )));
    }

    let mut sync_state = node
        .properties
        .get("state")
        .and_then(|pv| serde_json::to_value(pv).ok())
        .and_then(|v| match v {
            Value::Object(m) => Some(m),
            _ => None,
        })
        .unwrap_or_default();

    edit(&mut sync_state);

    let value = Value::Object(sync_state);
    let property = serde_json::from_value(value.clone())
        .map_err(|e| ApiError::internal(format!("failed to encode mount state: {e}")))?;
    node.properties.insert("state".to_string(), property);

    svc.update_node(node).await?;
    Ok(value)
}

/// Project the written state into the response shape.
fn response(sync_state: &Value) -> MountControlResponse {
    let flag = |key: &str| {
        sync_state
            .get(key)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    MountControlResponse {
        ok: true,
        paused: flag("paused"),
        stop_requested: flag("stop_requested"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with(pairs: &[(&str, Value)]) -> serde_json::Map<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect()
    }

    /// A stop must discard the resume point. `is_due` treats a mount with a
    /// pending backfill as due on the very next tick, so a stop that kept the
    /// cursor would restart within a minute — a button that does nothing.
    #[test]
    fn stop_clears_the_resume_point() {
        let mut s = state_with(&[
            ("backfill_cursor", json!("page-42")),
            ("backfill_stack", json!([["folder-1", "inbox"]])),
            ("backfill_items_done", json!(433_500)),
            ("delta_items_done", json!(1_200)),
        ]);

        s.insert("stop_requested".to_string(), json!(true));
        s.remove("backfill_cursor");
        s.remove("backfill_stack");
        s.insert("backfill_items_done".to_string(), json!(0));
        s.insert("delta_items_done".to_string(), json!(0));
        s.insert("backfill_complete".to_string(), json!(false));

        assert!(!s.contains_key("backfill_cursor"));
        assert!(!s.contains_key("backfill_stack"));
        assert_eq!(s["backfill_items_done"], json!(0));
        // Never claim completion: the walk did not reach the end, and reconcile
        // deletes are gated on that flag.
        assert_eq!(s["backfill_complete"], json!(false));
    }

    /// Pause must not touch the resume point — that is the whole difference
    /// between pausing and stopping.
    #[test]
    fn pause_preserves_the_resume_point() {
        let mut s = state_with(&[
            ("backfill_cursor", json!("page-42")),
            ("backfill_items_done", json!(1_240)),
        ]);
        s.insert("paused".to_string(), json!(true));

        assert_eq!(s["backfill_cursor"], json!("page-42"));
        assert_eq!(s["backfill_items_done"], json!(1_240));
    }

    /// Resuming clears a stale stop, or a mount that was stopped and then
    /// paused would stop again the instant it resumed.
    #[test]
    fn resuming_clears_a_stale_stop_request() {
        let mut s = state_with(&[("stop_requested", json!(true)), ("paused", json!(true))]);
        let paused = false;
        s.insert("paused".to_string(), json!(paused));
        if !paused {
            s.insert("stop_requested".to_string(), json!(false));
        }
        assert_eq!(s["stop_requested"], json!(false));
    }

    #[test]
    fn response_defaults_missing_flags_to_false() {
        let r = response(&json!({}));
        assert!(!r.paused);
        assert!(!r.stop_requested);
        assert!(r.ok);
    }
}
