// SPDX-License-Identifier: BSL-1.1

//! Revision-history retention and garbage collection for one repository.
//!
//! `POST …/history/gc` prunes superseded MVCC versions (keeping HEAD, every
//! revision inside the retention window, and every tag and branch fork point),
//! deletes the blobs only the pruned versions referenced, and compacts the
//! column families it touched. It reports what it removed and the SST bytes
//! the database gave back. `GET|PUT|DELETE …/history/retention` read and set
//! the stored policy for the repository (`?branch=` for one branch).

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use raisin_rocksdb::management::history_gc::{
    self, configured_options, retention, GcRunOutcome, HistoryRetention,
};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

use super::types::ErrorResponse;

type HandlerError = (StatusCode, Json<ErrorResponse>);

fn err(status: StatusCode, msg: impl Into<String>) -> HandlerError {
    (status, Json(ErrorResponse { error: msg.into() }))
}

fn storage(
    state: &AppState,
) -> Result<&std::sync::Arc<raisin_rocksdb::RocksDBStorage>, HandlerError> {
    state.rocksdb_storage.as_ref().ok_or_else(|| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "RocksDB storage not initialized",
        )
    })
}

/// Request body for a repository GC run. All fields are optional.
#[derive(Debug, Default, Deserialize)]
pub struct HistoryGcRequest {
    /// Report what would be removed without removing anything.
    #[serde(default)]
    pub dry_run: bool,
    /// Use this retention for every branch instead of the stored policies.
    #[serde(default)]
    pub keep_days: Option<u32>,
    #[serde(default)]
    pub keep_revisions: Option<u64>,
    /// Compact afterwards (default true).
    #[serde(default)]
    pub compact: Option<bool>,
}

/// Run history GC for one repository.
///
/// POST /api/admin/management/database/{tenant}/{repo}/history/gc
pub async fn run_repository_history_gc(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    body: Option<Json<HistoryGcRequest>>,
) -> Result<Json<GcRunOutcome>, HandlerError> {
    let storage = storage(&state)?.clone();
    let req = body.map(|Json(b)| b).unwrap_or_default();

    let mut opts = configured_options(&storage);
    opts.tenant = Some(tenant.clone());
    opts.repo = Some(repo.clone());
    opts.dry_run = req.dry_run;
    if let Some(compact) = req.compact {
        opts.compact = compact;
    }
    if req.keep_days.is_some() || req.keep_revisions.is_some() {
        opts.retention_override = Some(HistoryRetention {
            keep_days: req.keep_days,
            keep_revisions: req.keep_revisions,
        });
    }

    tracing::warn!(
        tenant = %tenant,
        repo = %repo,
        dry_run = opts.dry_run,
        retention_override = ?opts.retention_override,
        "History GC requested (admin action)"
    );

    history_gc::run_gc_and_sweep_blobs(storage, state.bin.as_ref(), opts)
        .await
        .map(Json)
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("History GC failed: {e}"),
            )
        })
}

/// `?branch=` selects one branch; without it the repository-wide policy.
#[derive(Debug, Deserialize)]
pub struct RetentionQuery {
    pub branch: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RetentionResponse {
    pub tenant: String,
    pub repo: String,
    /// `*` for the repository-wide policy.
    pub branch: String,
    /// The policy stored at exactly this level, if any.
    pub stored: Option<HistoryRetention>,
    /// What GC applies here after falling back to the repository policy and
    /// the server default.
    pub effective: HistoryRetention,
}

fn retention_response(
    storage: &raisin_rocksdb::RocksDBStorage,
    tenant: String,
    repo: String,
    branch: String,
) -> Result<RetentionResponse, HandlerError> {
    let internal = |e: raisin_error::Error| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
    let stored = retention::get_policy(storage.db(), &tenant, &repo, &branch).map_err(internal)?;
    let effective = retention::resolve_policy(
        storage.db(),
        &tenant,
        &repo,
        &branch,
        storage.config().history_retention,
    )
    .map_err(internal)?;
    Ok(RetentionResponse {
        tenant,
        repo,
        branch,
        stored,
        effective,
    })
}

/// GET /api/admin/management/database/{tenant}/{repo}/history/retention
pub async fn get_history_retention(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(q): Query<RetentionQuery>,
) -> Result<Json<RetentionResponse>, HandlerError> {
    let storage = storage(&state)?;
    let branch = q
        .branch
        .unwrap_or_else(|| retention::ALL_BRANCHES.to_string());
    retention_response(storage, tenant, repo, branch).map(Json)
}

/// PUT /api/admin/management/database/{tenant}/{repo}/history/retention
///
/// Body: `{"keep_days": 30, "keep_revisions": 500}`; `{}` keeps everything.
pub async fn put_history_retention(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(q): Query<RetentionQuery>,
    Json(policy): Json<HistoryRetention>,
) -> Result<Json<RetentionResponse>, HandlerError> {
    let storage = storage(&state)?;
    let branch = q
        .branch
        .unwrap_or_else(|| retention::ALL_BRANCHES.to_string());
    retention::set_policy(storage.db(), &tenant, &repo, &branch, Some(&policy))
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    tracing::info!(tenant = %tenant, repo = %repo, branch = %branch, ?policy, "History retention set");
    retention_response(storage, tenant, repo, branch).map(Json)
}

/// DELETE /api/admin/management/database/{tenant}/{repo}/history/retention
pub async fn delete_history_retention(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(q): Query<RetentionQuery>,
) -> Result<Json<RetentionResponse>, HandlerError> {
    let storage = storage(&state)?;
    let branch = q
        .branch
        .unwrap_or_else(|| retention::ALL_BRANCHES.to_string());
    retention::set_policy(storage.db(), &tenant, &repo, &branch, None)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    retention_response(storage, tenant, repo, branch).map(Json)
}
