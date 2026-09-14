// SPDX-License-Identifier: BSL-1.1

//! Vector (HNSW) index management handlers.
//!
//! Endpoints for verifying, rebuilding, optimizing, restoring, and checking
//! health of HNSW-based vector search indexes.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};

use raisin_storage::jobs::JobType;

use crate::state::AppState;

use super::types::{get_branch_name, DatabaseOpQuery, ErrorResponse, JobResponse};

/// Queue a vector maintenance job so the worker pool runs it.
///
/// Context first, then `register_job_with_id` on the storage's registry — the
/// same order as fulltext maintenance. The old handlers registered a job with
/// no context and ran the work in a detached task; the worker claimed the job,
/// found no context and marked it failed (verify), or the task's status raced
/// the worker's (rebuild). `MaintenanceJobHandler` now runs it.
#[cfg(feature = "storage-rocksdb")]
pub(super) async fn queue_vector_job(
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
    if state.hnsw_management.is_none() {
        return Err(internal("HNSW management not initialized".to_string()));
    }
    let rocksdb_storage = state
        .rocksdb_storage
        .as_ref()
        .ok_or_else(|| internal("RocksDB storage not initialized".to_string()))?;

    let context = JobContext {
        tenant_id: tenant.to_string(),
        repo_id: repo.to_string(),
        branch: branch.to_string(),
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

/// Verify vector index integrity.
///
/// POST /api/admin/management/database/:tenant/:repo/vector/verify
#[cfg(feature = "storage-rocksdb")]
pub async fn verify_vector_index(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(params): Query<DatabaseOpQuery>,
) -> Result<Json<JobResponse>, (StatusCode, Json<ErrorResponse>)> {
    let branch = get_branch_name(&state, &tenant, &repo, params.branch).await?;

    tracing::info!(
        "Starting vector index verification for {}/{}/{}",
        tenant,
        repo,
        branch
    );

    let job_id = queue_vector_job(
        &state,
        JobType::VectorVerify,
        &tenant,
        &repo,
        &branch,
        Default::default(),
    )
    .await?;

    Ok(Json(JobResponse {
        job_id: job_id.0,
        message: format!(
            "Vector verification started for {}/{}/{}",
            tenant, repo, branch
        ),
    }))
}

/// Rebuild vector index from scratch.
///
/// POST /api/admin/management/database/:tenant/:repo/vector/rebuild
#[cfg(feature = "storage-rocksdb")]
pub async fn rebuild_vector_index(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(params): Query<DatabaseOpQuery>,
) -> Result<Json<JobResponse>, (StatusCode, Json<ErrorResponse>)> {
    let branch = get_branch_name(&state, &tenant, &repo, params.branch).await?;

    tracing::info!(
        "Starting vector index rebuild for {}/{}/{}",
        tenant,
        repo,
        branch
    );

    let job_id = queue_vector_job(
        &state,
        JobType::VectorRebuild,
        &tenant,
        &repo,
        &branch,
        Default::default(),
    )
    .await?;

    Ok(Json(JobResponse {
        job_id: job_id.0,
        message: format!("Vector rebuild started for {}/{}/{}", tenant, repo, branch),
    }))
}

/// Optimize vector index structure.
///
/// POST /api/admin/management/database/:tenant/:repo/vector/optimize
///
/// Note: HNSW does not require optimization like Tantivy. This is a no-op
/// for API completeness.
#[cfg(feature = "storage-rocksdb")]
pub async fn optimize_vector_index(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(params): Query<DatabaseOpQuery>,
) -> Result<Json<JobResponse>, (StatusCode, Json<ErrorResponse>)> {
    let branch = get_branch_name(&state, &tenant, &repo, params.branch).await?;

    tracing::info!(
        "Vector index optimization requested for {}/{}/{} (no-op for HNSW)",
        tenant,
        repo,
        branch
    );

    let job_id = queue_vector_job(
        &state,
        JobType::VectorOptimize,
        &tenant,
        &repo,
        &branch,
        Default::default(),
    )
    .await?;

    Ok(Json(JobResponse {
        job_id: job_id.0,
        message: format!(
            "Vector optimization started (no-op for HNSW) for {}/{}/{}",
            tenant, repo, branch
        ),
    }))
}

/// Restore vector index from embeddings stored in RocksDB.
///
/// POST /api/admin/management/database/:tenant/:repo/vector/restore
///
/// This rebuilds the HNSW index from the embeddings column family.
/// Use this after HNSW index files are lost or corrupted, or when
/// restoring from a backup that only includes RocksDB data.
///
/// Functionally equivalent to rebuild, but semantically indicates
/// disaster recovery rather than routine maintenance.
#[cfg(feature = "storage-rocksdb")]
pub async fn restore_vector_index(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(params): Query<DatabaseOpQuery>,
) -> Result<Json<JobResponse>, (StatusCode, Json<ErrorResponse>)> {
    let branch = get_branch_name(&state, &tenant, &repo, params.branch).await?;

    tracing::info!(
        "Starting vector index restore for {}/{}/{}",
        tenant,
        repo,
        branch
    );

    let job_id = queue_vector_job(
        &state,
        JobType::VectorRebuild,
        &tenant,
        &repo,
        &branch,
        Default::default(),
    )
    .await?;

    Ok(Json(JobResponse {
        job_id: job_id.0,
        message: format!(
            "Vector index restore started for {}/{}/{}. Rebuilding from stored embeddings.",
            tenant, repo, branch
        ),
    }))
}

/// Get vector index health.
///
/// GET /api/admin/management/database/:tenant/:repo/vector/health
#[cfg(feature = "storage-rocksdb")]
pub async fn get_vector_health(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(params): Query<DatabaseOpQuery>,
) -> Result<Json<raisin_storage::IndexHealth>, (StatusCode, Json<ErrorResponse>)> {
    let hnsw_mgmt = match &state.hnsw_management {
        Some(mgmt) => mgmt,
        None => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "HNSW management not initialized".to_string(),
                }),
            ));
        }
    };

    let branch = get_branch_name(&state, &tenant, &repo, params.branch).await?;

    match hnsw_mgmt.get_health(&tenant, &repo, &branch).await {
        Ok(health) => Ok(Json(health)),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to get health: {}", e),
            }),
        )),
    }
}
