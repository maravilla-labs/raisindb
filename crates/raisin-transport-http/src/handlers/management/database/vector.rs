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
use std::sync::Arc;

use raisin_storage::jobs::JobType;

use crate::state::AppState;

use super::types::{get_branch_name, DatabaseOpQuery, ErrorResponse, JobResponse};

/// Verify vector index integrity.
///
/// POST /api/admin/management/database/:tenant/:repo/vector/verify
#[cfg(feature = "storage-rocksdb")]
pub async fn verify_vector_index(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(params): Query<DatabaseOpQuery>,
) -> Result<Json<JobResponse>, (StatusCode, Json<ErrorResponse>)> {
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

    let job_registry = rocksdb_storage.job_registry();
    let branch = get_branch_name(&state, &tenant, &repo, params.branch).await?;

    tracing::info!(
        "Starting vector index verification for {}/{}/{}",
        tenant,
        repo,
        branch
    );

    let job_id = job_registry
        .register_job(
            JobType::VectorVerify,
            Some(tenant.clone()),
            None,
            None,
            None,
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to register job: {}", e),
                }),
            )
        })?;

    let mgmt = Arc::clone(hnsw_mgmt);
    let job_registry_clone = Arc::clone(job_registry);
    let job_id_clone = job_id.clone();
    let tenant_clone = tenant.clone();
    let repo_clone = repo.clone();
    let branch_clone = branch.clone();
    tokio::spawn(async move {
        let _ = job_registry_clone.mark_running(&job_id_clone).await;

        match mgmt
            .verify_index(&tenant_clone, &repo_clone, &branch_clone)
            .await
        {
            Ok(report) => {
                let result_json = serde_json::to_value(&report).unwrap_or_default();
                let _ = job_registry_clone
                    .set_result(&job_id_clone, result_json)
                    .await;
                let _ = job_registry_clone.mark_completed(&job_id_clone).await;

                tracing::info!(
                    "Vector verification completed for {}/{}/{}: {:?}",
                    tenant_clone,
                    repo_clone,
                    branch_clone,
                    report.status
                );
            }
            Err(e) => {
                let _ = job_registry_clone
                    .mark_failed(&job_id_clone, e.to_string())
                    .await;
                tracing::error!("Vector verification failed: {}", e);
            }
        }
    });

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

    let job_registry = rocksdb_storage.job_registry();
    let branch = get_branch_name(&state, &tenant, &repo, params.branch).await?;

    tracing::info!(
        "Starting vector index rebuild for {}/{}/{}",
        tenant,
        repo,
        branch
    );

    let job_id = job_registry
        .register_job(
            JobType::VectorRebuild,
            Some(tenant.clone()),
            None,
            None,
            None,
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to register job: {}", e),
                }),
            )
        })?;

    let mgmt = Arc::clone(hnsw_mgmt);
    let job_registry_clone = Arc::clone(job_registry);
    let job_id_clone = job_id.clone();
    let tenant_clone = tenant.clone();
    let repo_clone = repo.clone();
    let branch_clone = branch.clone();
    tokio::spawn(async move {
        let _ = job_registry_clone.mark_running(&job_id_clone).await;

        match mgmt
            .rebuild_index(
                &tenant_clone,
                &repo_clone,
                &branch_clone,
                Some(job_id_clone.clone()),
            )
            .await
        {
            Ok(stats) => {
                let result_json = serde_json::to_value(&stats).unwrap_or_default();
                let _ = job_registry_clone
                    .set_result(&job_id_clone, result_json)
                    .await;
                let _ = job_registry_clone.mark_completed(&job_id_clone).await;

                tracing::info!(
                    "Vector rebuild completed for {}/{}/{}: {} items processed",
                    tenant_clone,
                    repo_clone,
                    branch_clone,
                    stats.items_processed
                );
            }
            Err(e) => {
                let _ = job_registry_clone
                    .mark_failed(&job_id_clone, e.to_string())
                    .await;
                tracing::error!("Vector rebuild failed: {}", e);
            }
        }
    });

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

    let job_registry = rocksdb_storage.job_registry();
    let branch = get_branch_name(&state, &tenant, &repo, params.branch).await?;

    tracing::info!(
        "Vector index optimization requested for {}/{}/{} (no-op for HNSW)",
        tenant,
        repo,
        branch
    );

    let job_id = job_registry
        .register_job(
            JobType::VectorOptimize,
            Some(tenant.clone()),
            None,
            None,
            None,
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to register job: {}", e),
                }),
            )
        })?;

    let mgmt = Arc::clone(hnsw_mgmt);
    let job_registry_clone = Arc::clone(job_registry);
    let job_id_clone = job_id.clone();
    let tenant_clone = tenant.clone();
    let repo_clone = repo.clone();
    let branch_clone = branch.clone();
    tokio::spawn(async move {
        let _ = job_registry_clone.mark_running(&job_id_clone).await;

        match mgmt
            .optimize_index(&tenant_clone, &repo_clone, &branch_clone)
            .await
        {
            Ok(stats) => {
                let result_json = serde_json::to_value(&stats).unwrap_or_default();
                let _ = job_registry_clone
                    .set_result(&job_id_clone, result_json)
                    .await;
                let _ = job_registry_clone.mark_completed(&job_id_clone).await;

                tracing::info!(
                    "Vector optimization completed (no-op) for {}/{}/{}",
                    tenant_clone,
                    repo_clone,
                    branch_clone
                );
            }
            Err(e) => {
                let _ = job_registry_clone
                    .mark_failed(&job_id_clone, e.to_string())
                    .await;
                tracing::error!("Vector optimization failed: {}", e);
            }
        }
    });

    Ok(Json(JobResponse {
        job_id: job_id.0,
        message: format!(
            "Vector optimization started (no-op for HNSW) for {}/{}/{}",
            tenant, repo, branch
        ),
    }))
}

/// Restore vector index from backup.
///
/// POST /api/admin/management/database/:tenant/:repo/vector/restore
#[cfg(feature = "storage-rocksdb")]
pub async fn restore_vector_index(
    State(_state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(params): Query<DatabaseOpQuery>,
) -> Result<Json<JobResponse>, (StatusCode, Json<ErrorResponse>)> {
    let branch = params.branch.unwrap_or_else(|| "main".to_string());

    tracing::warn!(
        "Vector index restore not yet implemented for {}/{}/{}",
        tenant,
        repo,
        branch
    );

    Err((
        StatusCode::NOT_IMPLEMENTED,
        Json(ErrorResponse {
            error: "Vector index operations not yet implemented (Phase 3)".to_string(),
        }),
    ))
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
