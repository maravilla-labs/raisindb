// SPDX-License-Identifier: BSL-1.1

//! `oauth/disconnect` and `mounts/{mount_id}/sync` admin handlers.

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use raisin_crypto::SecretBox;
use raisin_models::auth::AuthContext;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    connected_accounts, decrypt_client_secret, json_str, oauth_config, require_admin,
    set_connected_accounts,
};
use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::AppState;

/// Request body for `oauth/disconnect`.
#[derive(Debug, Deserialize)]
pub struct DisconnectRequest {
    pub integration_path: String,
    pub account_id: String,
    /// Disconnect even though mounts are pinned to this connection.
    ///
    /// Without it, a disconnect that would orphan a mount is refused. The mounts
    /// do not fail at disconnect time — they fail on their next sync, once a
    /// minute, with `connected account ... no longer exists on this connector`,
    /// and there is nothing in that message tying it back to the click that
    /// caused it. Re-connecting does not undo it either: consent mints a NEW
    /// account id, so the pinned mounts stay broken while the console shows a
    /// healthy connection.
    #[serde(default)]
    pub force: bool,
}

/// Response for `oauth/disconnect`.
#[derive(Debug, Serialize)]
pub struct DisconnectResponse {
    pub disconnected: bool,
    /// Mounts that were pinned to this connection and are now unbound. Present
    /// only on a forced disconnect; each needs a `mounts/{id}/rebind` before it
    /// will sync again.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub orphaned_mounts: Vec<String>,
}

/// Best-effort revoke the provider tokens for one connected account and drop the
/// account entry from the integration node.
pub async fn disconnect(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<DisconnectRequest>,
) -> Result<Json<DisconnectResponse>, ApiError> {
    require_admin(auth.as_deref())?;

    let master_key = state.get_master_key().ok();
    let svc = super::config_service(&state, &tenant.tenant_id, &repo, "integration-oauth");
    let node = svc
        .get_by_path(&req.integration_path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(req.integration_path.clone()))?;

    let cfg = oauth_config(&node);
    let revoke_url = json_str(&cfg, "revoke_url");

    // Locate the target account (if any).
    let target = connected_accounts(&node)
        .into_iter()
        .find(|a| a.get("id").and_then(|v| v.as_str()) == Some(req.account_id.as_str()));

    // Refuse to orphan mounts silently. Checked BEFORE the revoke, so a refused
    // disconnect leaves the provider-side grant intact too — a half-done
    // disconnect that revoked the tokens but kept the entry would be the worst
    // of both.
    let pinned = mounts_pinned_to(&state, &tenant.tenant_id, &repo, &req.account_id).await?;
    if !pinned.is_empty() && !req.force {
        return Err(ApiError::new(
            axum::http::StatusCode::CONFLICT,
            "CONNECTION_IN_USE",
            format!(
                "{} mount(s) are pinned to this connection and will stop syncing if it is \
                 removed ({}). Rebind them to another connection first, or repeat with \
                 force to disconnect anyway.",
                pinned.len(),
                pinned.join(", ")
            ),
        ));
    }

    // Best-effort provider revoke; failures never block local removal. Done
    // BEFORE the lease is taken — it is a network round trip, and the lease
    // exists to cover a read-modify-write, not an HTTP call.
    if let (Some(revoke_url), Some(account), Some(key)) = (revoke_url, target, master_key) {
        if let Some(token) = access_token_for(&account, &key) {
            let _ = reqwest::Client::new()
                .post(&revoke_url)
                .form(&[("token", token.as_str())])
                .send()
                .await;
        }
    }

    // Removal re-reads inside the lease: `node` above predates the revoke call,
    // and writing it back would restore any token blob the refresh job rotated
    // meanwhile onto the OTHER connections of this connector.
    let account_id = req.account_id.clone();
    let removed = super::accounts_lock::with_accounts_lock(
        &state,
        &tenant.tenant_id,
        &repo,
        &req.integration_path,
        "integration-oauth",
        move |node| {
            let mut accounts = connected_accounts(node);
            let before = accounts.len();
            accounts.retain(|a| a.get("id").and_then(|v| v.as_str()) != Some(account_id.as_str()));
            let removed = accounts.len() != before;
            if removed {
                set_connected_accounts(node, accounts)?;
            }
            Ok(removed)
        },
    )
    .await?;

    if removed && !pinned.is_empty() {
        tracing::warn!(
            account_id = %req.account_id,
            orphaned = pinned.len(),
            "forced disconnect orphaned mounts; they need rebinding before they sync again"
        );
    }

    Ok(Json(DisconnectResponse {
        disconnected: removed,
        orphaned_mounts: if removed { pinned } else { Vec::new() },
    }))
}

/// Mount ids whose `account_ref` pins them to `account_id`.
///
/// Only EXPLICIT references count. A mount with no `account_ref` resolves to the
/// connector's single connection, so removing that connection breaks it too —
/// but it is equally repaired by connecting another, needs no rebind, and
/// blocking on it would make the last connection undeletable.
async fn mounts_pinned_to(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    account_id: &str,
) -> Result<Vec<String>, ApiError> {
    let svc = super::config_service(state, tenant_id, repo, "integration-oauth");
    let mounts = svc.list_by_type("raisin:VirtualMount").await?;
    Ok(mounts
        .into_iter()
        .filter(|m| super::json_prop(m, "account_ref").as_str() == Some(account_id))
        .map(|m| m.id)
        .collect())
}

/// Decrypt just the access token from an account's `tokens_encrypted` blob.
fn access_token_for(account: &Value, master_key: &[u8; 32]) -> Option<String> {
    let enc = account.get("tokens_encrypted").and_then(|v| v.as_str())?;
    let tokens = SecretBox::new(master_key).decrypt_json(enc).ok()?;
    tokens
        .get("access_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Request body for the manual sync endpoint.
#[derive(Debug, Default, Deserialize)]
pub struct SyncRequest {
    /// "delta" (default) or "full".
    pub mode: Option<String>,
}

/// Response for the manual sync endpoint.
#[derive(Debug, Serialize)]
pub struct SyncResponse {
    /// Job id of the enqueued sync, or `None` if an in-flight sync was deduped.
    pub job_id: Option<String>,
    pub status: String,
}

/// Enqueue a one-shot `VirtualMountSync` for a mount ("sync now").
#[cfg(feature = "storage-rocksdb")]
pub async fn sync_mount(
    State(state): State<AppState>,
    Path((repo, mount_id)): Path<(String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    body: Option<Json<SyncRequest>>,
) -> Result<Json<SyncResponse>, ApiError> {
    require_admin(auth.as_deref())?;

    let rocksdb = state
        .rocksdb_storage
        .as_ref()
        .ok_or_else(|| ApiError::internal("RocksDB backend required for mount sync"))?;

    let mode = body
        .and_then(|Json(b)| b.mode)
        .unwrap_or_else(|| "delta".to_string());
    // Validated rather than passed through: the engine treats any unrecognised
    // mode as `delta`, so a typo would quietly run the wrong kind of sync and
    // report success — particularly bad for `remap`, where the operator is
    // waiting for a migration that never happens.
    if !matches!(mode.as_str(), "delta" | "full" | "remap") {
        return Err(ApiError::validation_failed(format!(
            "unknown sync mode '{mode}'; expected one of: delta, full, remap"
        )));
    }

    let job_registry = rocksdb.job_registry();
    let job_data_store = rocksdb.job_data_store();

    let job_type = raisin_storage::jobs::JobType::VirtualMountSync {
        mount_id: mount_id.clone(),
        mode,
        trigger: "manual".to_string(),
    };
    let job_id = raisin_storage::jobs::JobId::new();
    let context = raisin_storage::jobs::JobContext {
        tenant_id: tenant.tenant_id.clone(),
        repo_id: repo.clone(),
        // The repo's real config branch, not a hardcoded "main": the scheduler
        // uses `repository.config.default_branch`, and a mismatch here builds a
        // different lock key and job context than the scheduled path, after
        // which the engine cannot find the mount and silently no-ops.
        branch: super::config_branch(&state, &tenant.tenant_id, &repo).await,
        workspace_id: "raisin:system".to_string(),
        revision: raisin_hlc::HLC::now(),
        metadata: std::collections::HashMap::new(),
    };
    job_data_store
        .put(&job_id, &context)
        .map_err(|e| ApiError::internal(format!("failed to store sync job context: {e}")))?;

    let registered = job_registry
        .register_job_with_id_idempotent(
            job_id.clone(),
            job_type,
            tenant.tenant_id.clone(),
            format!("vmount-sync:{}", mount_id),
            None,
        )
        .await
        .map_err(|e| ApiError::internal(format!("failed to enqueue sync job: {e}")))?;

    if registered {
        Ok(Json(SyncResponse {
            job_id: Some(job_id.0),
            status: "queued".to_string(),
        }))
    } else {
        Ok(Json(SyncResponse {
            job_id: None,
            status: "already_running".to_string(),
        }))
    }
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn sync_mount(
    State(_state): State<AppState>,
    Path((_repo, _mount_id)): Path<(String, String)>,
    Extension(_tenant): Extension<TenantInfo>,
    _auth: Option<Extension<AuthContext>>,
    _body: Option<Json<SyncRequest>>,
) -> Result<Json<SyncResponse>, ApiError> {
    Err(ApiError::internal(
        "Mount sync requires the RocksDB backend",
    ))
}
