// SPDX-License-Identifier: BSL-1.1

//! Fulltext index management handlers.
//!
//! Endpoints for verifying, rebuilding, optimizing, purging, and checking
//! health of Tantivy-based fulltext search indexes.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};

use raisin_storage::jobs::JobType;

use crate::state::AppState;

use super::types::{get_branch_name, DatabaseOpQuery, ErrorResponse, JobResponse};

/// Queue one operator-triggered fulltext maintenance job and hand its id back.
///
/// The work itself runs in the worker (`FulltextJobHandler::handle_maintenance`),
/// NOT in a detached `tokio::spawn` here. That split is what made these
/// endpoints lie: `register_job` broadcasts the job to dispatch immediately, a
/// worker claimed it, found no `JobContext` — because the old code never wrote
/// one — and marked it `Failed: Job context not found`, overwriting the status
/// the detached task was maintaining. Both jobs of a production run reported
/// Failed while succeeding, and `FulltextOptimize` is the documented mitigation
/// for a Tantivy merge storm, i.e. exactly the moment an operator must be able
/// to trust the status field.
///
/// So: write the context FIRST, then register under that same id
/// (`register_job_with_id`), so dispatch can never observe a contextless job.
/// `max_retries = 0` — a rebuild or a purge is an operator decision, and
/// silently re-running it three times is not the job system's call to make.
#[cfg(feature = "storage-rocksdb")]
async fn queue_fulltext_job(
    state: &AppState,
    job_type: JobType,
    tenant: &str,
    repo: &str,
    branch: &str,
    metadata: std::collections::HashMap<String, serde_json::Value>,
) -> Result<raisin_storage::jobs::JobId, (StatusCode, Json<ErrorResponse>)> {
    use raisin_storage::jobs::{JobContext, JobId};

    let internal = |msg: String| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: msg }),
        )
    };

    // Both are `Some` exactly when the fulltext subsystem is up; the worker's
    // handler owns its own engine, so these are preconditions, not inputs.
    if state.tantivy_management.is_none() || state.indexing_engine.is_none() {
        return Err(internal("Tantivy management not initialized".to_string()));
    }
    let rocksdb_storage = state
        .rocksdb_storage
        .as_ref()
        .ok_or_else(|| internal("RocksDB storage not initialized".to_string()))?;

    let context = JobContext {
        tenant_id: tenant.to_string(),
        repo_id: repo.to_string(),
        branch: branch.to_string(),
        // Fulltext maintenance is branch-wide: it walks every workspace on the
        // branch rather than being scoped to one.
        workspace_id: String::new(),
        revision: raisin_hlc::HLC::new(0, 0),
        metadata,
    };

    let job_id = JobId::new();
    rocksdb_storage
        .job_data_store()
        .put(&job_id, &context)
        .map_err(|e| internal(format!("Failed to store job context: {}", e)))?;

    rocksdb_storage
        .job_registry()
        .register_job_with_id(
            job_id.clone(),
            job_type,
            tenant.to_string(),
            None,
            None,
            Some(0),
        )
        .await
        .map_err(|e| internal(format!("Failed to register job: {}", e)))?;

    Ok(job_id)
}

/// Verify fulltext index integrity.
///
/// POST /api/admin/management/database/:tenant/:repo/fulltext/verify
#[cfg(feature = "storage-rocksdb")]
pub async fn verify_fulltext_index(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(params): Query<DatabaseOpQuery>,
) -> Result<Json<JobResponse>, (StatusCode, Json<ErrorResponse>)> {
    let branch = get_branch_name(&state, &tenant, &repo, params.branch).await?;

    tracing::info!(
        "Queueing fulltext index verification for {}/{}/{}",
        tenant,
        repo,
        branch
    );

    let job_id = queue_fulltext_job(
        &state,
        JobType::FulltextVerify,
        &tenant,
        &repo,
        &branch,
        Default::default(),
    )
    .await?;

    Ok(Json(JobResponse {
        job_id: job_id.0,
        message: format!(
            "Fulltext verification started for {}/{}/{}",
            tenant, repo, branch
        ),
    }))
}

/// Rebuild fulltext index from scratch.
///
/// POST /api/admin/management/database/:tenant/:repo/fulltext/rebuild
///
/// Holds the storage-shared `IndexLockManager` lock for the affected
/// `(tenant, repo, branch)` so concurrent batch indexing waits
/// instead of racing the directory. Returns honest
/// `RebuildStats.items_processed` once finished — the v0.1.28 stub
/// always returned 0 and lied about success.
#[cfg(feature = "storage-rocksdb")]
pub async fn rebuild_fulltext_index(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(params): Query<DatabaseOpQuery>,
) -> Result<Json<JobResponse>, (StatusCode, Json<ErrorResponse>)> {
    let branch = get_branch_name(&state, &tenant, &repo, params.branch).await?;

    tracing::info!(
        "Queueing fulltext index rebuild for {}/{}/{}",
        tenant,
        repo,
        branch
    );

    let job_id = queue_fulltext_job(
        &state,
        JobType::FulltextRebuild,
        &tenant,
        &repo,
        &branch,
        Default::default(),
    )
    .await?;

    Ok(Json(JobResponse {
        job_id: job_id.0,
        message: format!(
            "Fulltext rebuild started for {}/{}/{}",
            tenant, repo, branch
        ),
    }))
}

/// Reconcile the fulltext index against the canonical node store.
///
/// POST /api/admin/management/database/:tenant/:repo/fulltext/reconcile
///
/// The v0.1.29 recovery path for tenants impacted by v0.1.28's
/// aggregator-AND-gate bug. Unlike rebuild, this does not delete the
/// existing index — it just replays every node through
/// `do_batch_index` so missing entries get added. Tantivy's
/// `delete_term + add_document` makes the per-node ops idempotent so
/// running it on a healthy index is harmless (only wasted I/O).
#[cfg(feature = "storage-rocksdb")]
pub async fn reconcile_fulltext_index(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(params): Query<DatabaseOpQuery>,
) -> Result<Json<JobResponse>, (StatusCode, Json<ErrorResponse>)> {
    let branch = get_branch_name(&state, &tenant, &repo, params.branch).await?;

    tracing::info!(
        "Queueing fulltext index reconcile for {}/{}/{}",
        tenant,
        repo,
        branch
    );

    // Rebuild and reconcile share `JobType::FulltextRebuild`; the context
    // carries which one this is. Adding a job-type variant instead would break
    // every persisted job of the existing type on upgrade.
    let mut metadata = std::collections::HashMap::new();
    metadata.insert(
        raisin_rocksdb::META_FULLTEXT_MODE.to_string(),
        serde_json::json!(raisin_rocksdb::FULLTEXT_MODE_RECONCILE),
    );

    let job_id = queue_fulltext_job(
        &state,
        JobType::FulltextRebuild,
        &tenant,
        &repo,
        &branch,
        metadata,
    )
    .await?;

    Ok(Json(JobResponse {
        job_id: job_id.0,
        message: format!(
            "Fulltext reconcile started for {}/{}/{}",
            tenant, repo, branch
        ),
    }))
}

/// Optimize fulltext index (merge segments).
///
/// POST /api/admin/management/database/:tenant/:repo/fulltext/optimize
#[cfg(feature = "storage-rocksdb")]
pub async fn optimize_fulltext_index(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(params): Query<DatabaseOpQuery>,
) -> Result<Json<JobResponse>, (StatusCode, Json<ErrorResponse>)> {
    let branch = get_branch_name(&state, &tenant, &repo, params.branch).await?;

    tracing::info!(
        "Queueing fulltext index optimization for {}/{}/{}",
        tenant,
        repo,
        branch
    );

    let job_id = queue_fulltext_job(
        &state,
        JobType::FulltextOptimize,
        &tenant,
        &repo,
        &branch,
        Default::default(),
    )
    .await?;

    Ok(Json(JobResponse {
        job_id: job_id.0,
        message: format!(
            "Fulltext optimization started for {}/{}/{}",
            tenant, repo, branch
        ),
    }))
}

/// Purge fulltext index completely.
///
/// POST /api/admin/management/database/:tenant/:repo/fulltext/purge
#[cfg(feature = "storage-rocksdb")]
pub async fn purge_fulltext_index(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(params): Query<DatabaseOpQuery>,
) -> Result<Json<JobResponse>, (StatusCode, Json<ErrorResponse>)> {
    let branch = get_branch_name(&state, &tenant, &repo, params.branch).await?;

    tracing::warn!(
        "Queueing fulltext index purge for {}/{}/{}",
        tenant,
        repo,
        branch
    );

    let job_id = queue_fulltext_job(
        &state,
        JobType::FulltextPurge,
        &tenant,
        &repo,
        &branch,
        Default::default(),
    )
    .await?;

    Ok(Json(JobResponse {
        job_id: job_id.0,
        message: format!("Fulltext purge started for {}/{}/{}", tenant, repo, branch),
    }))
}

/// Snapshot of the in-memory fulltext error counter for the
/// requested `(tenant, repo, branch)`. Used by the admin console to
/// render an "Index Errors" card with kind-by-kind counts and the
/// most recent error message.
///
/// Counts persist for the process lifetime. They reset on restart
/// (intentional — the metric answers "is the system unhealthy *now*?"
/// not "has it ever been unhealthy"). Use `DELETE` on the same path
/// to drop the entry, e.g. after a successful rebuild/reconcile.
///
/// GET /api/admin/management/database/:tenant/:repo/fulltext/errors
#[cfg(feature = "storage-rocksdb")]
pub async fn get_fulltext_errors(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(params): Query<DatabaseOpQuery>,
) -> Result<Json<raisin_rocksdb::FulltextErrorStats>, (StatusCode, Json<ErrorResponse>)> {
    let rocksdb_storage = match &state.rocksdb_storage {
        Some(storage) => storage,
        None => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "RocksDB storage not initialized".to_string(),
                }),
            ));
        }
    };

    let branch = get_branch_name(&state, &tenant, &repo, params.branch).await?;
    let stats = rocksdb_storage
        .fulltext_error_counter()
        .snapshot(&tenant, &repo, &branch)
        .await;
    Ok(Json(stats))
}

/// Drop the in-memory error counter entry for the requested
/// `(tenant, repo, branch)`. Called by the admin console after a
/// successful rebuild/reconcile so the dashboard goes green without
/// needing a process restart.
///
/// DELETE /api/admin/management/database/:tenant/:repo/fulltext/errors
#[cfg(feature = "storage-rocksdb")]
pub async fn clear_fulltext_errors(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(params): Query<DatabaseOpQuery>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let rocksdb_storage = match &state.rocksdb_storage {
        Some(storage) => storage,
        None => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "RocksDB storage not initialized".to_string(),
                }),
            ));
        }
    };

    let branch = get_branch_name(&state, &tenant, &repo, params.branch).await?;
    rocksdb_storage
        .fulltext_error_counter()
        .clear(&tenant, &repo, &branch)
        .await;

    tracing::info!(tenant, repo, branch, "Cleared fulltext error counters");
    Ok(StatusCode::NO_CONTENT)
}

/// Get fulltext index health.
///
/// GET /api/admin/management/database/:tenant/:repo/fulltext/health
#[cfg(feature = "storage-rocksdb")]
pub async fn get_fulltext_health(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(params): Query<DatabaseOpQuery>,
) -> Result<Json<raisin_storage::IndexHealth>, (StatusCode, Json<ErrorResponse>)> {
    let tantivy_mgmt = match &state.tantivy_management {
        Some(mgmt) => mgmt,
        None => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Tantivy management not initialized".to_string(),
                }),
            ));
        }
    };

    let branch = get_branch_name(&state, &tenant, &repo, params.branch).await?;

    match tantivy_mgmt.get_health(&tenant, &repo, &branch).await {
        Ok(health) => Ok(Json(health)),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to get health: {}", e),
            }),
        )),
    }
}
