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
use std::sync::Arc;

use raisin_storage::jobs::JobType;

use crate::state::AppState;

use super::types::{get_branch_name, DatabaseOpQuery, ErrorResponse, JobResponse};

/// Verify fulltext index integrity.
///
/// POST /api/admin/management/database/:tenant/:repo/fulltext/verify
#[cfg(feature = "storage-rocksdb")]
pub async fn verify_fulltext_index(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(params): Query<DatabaseOpQuery>,
) -> Result<Json<JobResponse>, (StatusCode, Json<ErrorResponse>)> {
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
        "Starting fulltext index verification for {}/{}/{}",
        tenant,
        repo,
        branch
    );

    let job_id = job_registry
        .register_job(JobType::FulltextVerify, tenant.clone(), None, None, None)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to register job: {}", e),
                }),
            )
        })?;

    let mgmt = Arc::clone(tantivy_mgmt);
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
                    "Fulltext verification completed for {}/{}/{}: {:?}",
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
                tracing::error!("Fulltext verification failed: {}", e);
            }
        }
    });

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
    let indexing_engine = match &state.indexing_engine {
        Some(engine) => engine,
        None => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Tantivy indexing engine not initialized".to_string(),
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
        "Starting fulltext index rebuild for {}/{}/{}",
        tenant,
        repo,
        branch
    );

    let job_id = job_registry
        .register_job(JobType::FulltextRebuild, tenant.clone(), None, None, None)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to register job: {}", e),
                }),
            )
        })?;

    let storage_clone = Arc::clone(rocksdb_storage);
    let engine_clone = Arc::clone(indexing_engine);
    let job_registry_clone = Arc::clone(job_registry);
    let job_id_clone = job_id.clone();
    let tenant_clone = tenant.clone();
    let repo_clone = repo.clone();
    let branch_clone = branch.clone();
    tokio::spawn(async move {
        let _ = job_registry_clone.mark_running(&job_id_clone).await;

        match raisin_rocksdb::management::rebuild_fulltext_index(
            &storage_clone,
            &engine_clone,
            &tenant_clone,
            &repo_clone,
            &branch_clone,
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
                    "Fulltext rebuild completed for {}/{}/{}: {} items processed",
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
                tracing::error!("Fulltext rebuild failed: {}", e);
            }
        }
    });

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
    let indexing_engine = match &state.indexing_engine {
        Some(engine) => engine,
        None => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Tantivy indexing engine not initialized".to_string(),
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
        "Starting fulltext index reconcile for {}/{}/{}",
        tenant,
        repo,
        branch
    );

    let job_id = job_registry
        .register_job(JobType::FulltextRebuild, tenant.clone(), None, None, None)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to register job: {}", e),
                }),
            )
        })?;

    let storage_clone = Arc::clone(rocksdb_storage);
    let engine_clone = Arc::clone(indexing_engine);
    let job_registry_clone = Arc::clone(job_registry);
    let job_id_clone = job_id.clone();
    let tenant_clone = tenant.clone();
    let repo_clone = repo.clone();
    let branch_clone = branch.clone();
    tokio::spawn(async move {
        let _ = job_registry_clone.mark_running(&job_id_clone).await;

        match raisin_rocksdb::management::reconcile_fulltext_index(
            &storage_clone,
            &engine_clone,
            &tenant_clone,
            &repo_clone,
            &branch_clone,
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
                    "Fulltext reconcile completed for {}/{}/{}: {} items processed",
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
                tracing::error!("Fulltext reconcile failed: {}", e);
            }
        }
    });

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
        "Starting fulltext index optimization for {}/{}/{}",
        tenant,
        repo,
        branch
    );

    let job_id = job_registry
        .register_job(JobType::FulltextOptimize, tenant.clone(), None, None, None)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to register job: {}", e),
                }),
            )
        })?;

    let mgmt = Arc::clone(tantivy_mgmt);
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
                    "Fulltext optimization completed for {}/{}/{}: {} segments merged",
                    tenant_clone,
                    repo_clone,
                    branch_clone,
                    stats.segments_merged
                );
            }
            Err(e) => {
                let _ = job_registry_clone
                    .mark_failed(&job_id_clone, e.to_string())
                    .await;
                tracing::error!("Fulltext optimization failed: {}", e);
            }
        }
    });

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

    tracing::warn!(
        "Starting fulltext index purge for {}/{}/{}",
        tenant,
        repo,
        branch
    );

    let job_id = job_registry
        .register_job(JobType::FulltextPurge, tenant.clone(), None, None, None)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to register job: {}", e),
                }),
            )
        })?;

    let mgmt = Arc::clone(tantivy_mgmt);
    let job_registry_clone = Arc::clone(job_registry);
    let job_id_clone = job_id.clone();
    let tenant_clone = tenant.clone();
    let repo_clone = repo.clone();
    let branch_clone = branch.clone();
    tokio::spawn(async move {
        let _ = job_registry_clone.mark_running(&job_id_clone).await;

        match mgmt
            .purge_index(&tenant_clone, &repo_clone, &branch_clone)
            .await
        {
            Ok(()) => {
                let _ = job_registry_clone.mark_completed(&job_id_clone).await;
                tracing::info!(
                    "Fulltext purge completed for {}/{}/{}",
                    tenant_clone,
                    repo_clone,
                    branch_clone
                );
            }
            Err(e) => {
                let _ = job_registry_clone
                    .mark_failed(&job_id_clone, e.to_string())
                    .await;
                tracing::error!("Fulltext purge failed: {}", e);
            }
        }
    });

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
) -> Result<Json<raisin_rocksdb::FulltextErrorStats>, (StatusCode, Json<ErrorResponse>)>
{
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

    tracing::info!(
        tenant, repo, branch,
        "Cleared fulltext error counters"
    );
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
