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
}

/// Response for `oauth/disconnect`.
#[derive(Debug, Serialize)]
pub struct DisconnectResponse {
    pub disconnected: bool,
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
    let mut node = svc
        .get_by_path(&req.integration_path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(req.integration_path.clone()))?;

    let cfg = oauth_config(&node);
    let revoke_url = json_str(&cfg, "revoke_url");
    let mut accounts = connected_accounts(&node);

    // Locate the target account (if any).
    let target = accounts
        .iter()
        .find(|a| a.get("id").and_then(|v| v.as_str()) == Some(req.account_id.as_str()))
        .cloned();

    // Best-effort provider revoke; failures never block local removal.
    if let (Some(revoke_url), Some(account), Some(key)) = (revoke_url, target, master_key) {
        if let Some(token) = access_token_for(&account, &key) {
            let _ = reqwest::Client::new()
                .post(&revoke_url)
                .form(&[("token", token.as_str())])
                .send()
                .await;
        }
    }

    let before = accounts.len();
    accounts.retain(|a| a.get("id").and_then(|v| v.as_str()) != Some(req.account_id.as_str()));
    let removed = accounts.len() != before;

    if removed {
        set_connected_accounts(&mut node, accounts)?;
        svc.update_node(node).await?;
    }

    Ok(Json(DisconnectResponse {
        disconnected: removed,
    }))
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

    let job_registry = rocksdb.job_registry();
    let job_data_store = rocksdb.job_data_store();

    let job_type = raisin_storage::jobs::JobType::VirtualMountSync {
        mount_id: mount_id.clone(),
        mode,
    };
    let job_id = raisin_storage::jobs::JobId::new();
    let context = raisin_storage::jobs::JobContext {
        tenant_id: tenant.tenant_id.clone(),
        repo_id: repo.clone(),
        branch: "main".to_string(),
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
