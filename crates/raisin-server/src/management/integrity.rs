//! Integrity, index verification, rebuild, and orphan cleanup handlers.
//!
//! Each operation has a synchronous handler (generic over Storage) and an
//! asynchronous background-job variant (RocksDB-specific) that returns a job ID.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use raisin_storage::{BackgroundJobs, IndexType, ManagementOps};

use super::types::{ApiResponse, RebuildRequest};
use super::ManagementState;

// ---------------------------------------------------------------------------
// Integrity check
// ---------------------------------------------------------------------------

/// Synchronous integrity check for a tenant.
pub async fn check_integrity<S>(
    State(state): State<ManagementState<S>>,
    Path(tenant): Path<String>,
) -> Result<Json<ApiResponse<raisin_storage::IntegrityReport>>, StatusCode>
where
    S: ManagementOps + Send + Sync,
{
    match state.storage.check_integrity(&tenant).await {
        Ok(report) => Ok(Json(ApiResponse::ok(report))),
        Err(e) => {
            tracing::error!("Failed to check integrity for tenant {}: {}", tenant, e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Start integrity check as a background job (RocksDB only).
///
/// Returns immediately with a job ID that can be monitored via SSE.
#[cfg(feature = "storage-rocksdb")]
pub async fn start_integrity_check(
    State(state): State<ManagementState<raisin_rocksdb::RocksDBStorage>>,
    Path(tenant): Path<String>,
) -> Result<Json<ApiResponse<String>>, StatusCode> {
    use raisin_storage::jobs::{global_registry, JobStatus, JobType};

    tracing::info!("Starting async integrity check for tenant: {}", tenant);

    // Check if an integrity check is already running for this tenant
    let jobs = global_registry().list_jobs().await;
    for job in jobs {
        if matches!(job.job_type, JobType::IntegrityScan)
            && job.tenant.as_deref() == Some(&tenant)
            && matches!(job.status, JobStatus::Running | JobStatus::Executing | JobStatus::Scheduled)
        {
            tracing::info!(
                "Integrity check already running for tenant '{}', returning existing job ID",
                tenant
            );
            return Ok(Json(ApiResponse::ok(job.id.0)));
        }
    }

    // Register the job
    let job_id = match global_registry()
        .register_job(
            JobType::IntegrityScan,
            Some(tenant.clone()),
            None,
            None,
            None,
        )
        .await
    {
        Ok(id) => id,
        Err(e) => {
            tracing::error!("Failed to register integrity check job: {}", e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    // Spawn the job in the background
    let storage = state.storage.clone();
    let job_id_clone = job_id.clone();
    tokio::spawn(async move {
        // TODO: Re-implement when background jobs are available
        if false {
            let e: anyhow::Error = anyhow::anyhow!("Not implemented");
            tracing::error!("Integrity check job failed: {}", e);
        }
    });

    Ok(Json(ApiResponse::ok(job_id.0)))
}

/// Fallback for non-RocksDB storage backends.
#[cfg(not(feature = "storage-rocksdb"))]
pub async fn start_integrity_check<S>(
    State(_state): State<ManagementState<S>>,
    Path(_tenant): Path<String>,
) -> Result<Json<ApiResponse<String>>, StatusCode>
where
    S: ManagementOps + BackgroundJobs + Send + Sync,
{
    tracing::error!("Integrity check jobs are only supported with RocksDB storage");
    Err(StatusCode::NOT_IMPLEMENTED)
}

// ---------------------------------------------------------------------------
// Index verification
// ---------------------------------------------------------------------------

/// Synchronous index verification for a tenant.
pub async fn verify_indexes<S>(
    State(state): State<ManagementState<S>>,
    Path(tenant): Path<String>,
) -> Result<Json<ApiResponse<Vec<raisin_storage::IndexIssue>>>, StatusCode>
where
    S: ManagementOps + Send + Sync,
{
    match state.storage.verify_indexes(&tenant).await {
        Ok(issues) => Ok(Json(ApiResponse::ok(issues))),
        Err(e) => {
            tracing::error!("Failed to verify indexes for tenant {}: {}", tenant, e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Start index verification as a background job (RocksDB only).
#[cfg(feature = "storage-rocksdb")]
pub async fn start_verify_indexes(
    State(state): State<ManagementState<raisin_rocksdb::RocksDBStorage>>,
    Path(tenant): Path<String>,
) -> Result<Json<ApiResponse<String>>, StatusCode> {
    use raisin_storage::jobs::{global_registry, JobType};

    tracing::info!("Starting async index verification for tenant '{}'", tenant);

    let job_id = match global_registry()
        .register_job(JobType::IndexVerify, Some(tenant.clone()), None, None, None)
        .await
    {
        Ok(id) => id,
        Err(e) => {
            tracing::error!("Failed to register index verify job: {}", e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    let storage = state.storage.clone();
    let job_id_clone = job_id.clone();
    tokio::spawn(async move {
        // TODO: Re-implement when background jobs are available
        if false {
            let e: anyhow::Error = anyhow::anyhow!("Not implemented");
            tracing::error!("Index verify job failed: {}", e);
        }
    });

    Ok(Json(ApiResponse::ok(job_id.0)))
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn start_verify_indexes<S>(
    State(_state): State<ManagementState<S>>,
    Path(_tenant): Path<String>,
) -> Result<Json<ApiResponse<String>>, StatusCode>
where
    S: ManagementOps + BackgroundJobs + Send + Sync,
{
    tracing::error!("Async index verify jobs are only supported with RocksDB storage");
    Err(StatusCode::NOT_IMPLEMENTED)
}

// ---------------------------------------------------------------------------
// Index rebuild
// ---------------------------------------------------------------------------

/// Synchronous index rebuild for a tenant.
pub async fn rebuild_indexes<S>(
    State(state): State<ManagementState<S>>,
    Path(tenant): Path<String>,
    Json(req): Json<RebuildRequest>,
) -> Result<Json<ApiResponse<raisin_storage::RebuildStats>>, StatusCode>
where
    S: ManagementOps + Send + Sync,
{
    let index_type = match parse_index_type(&req.index_type) {
        Ok(t) => t,
        Err(msg) => return Ok(Json(ApiResponse::err(msg))),
    };

    match state.storage.rebuild_indexes(&tenant, index_type).await {
        Ok(stats) => Ok(Json(ApiResponse::ok(stats))),
        Err(e) => {
            tracing::error!("Failed to rebuild indexes for tenant {}: {}", tenant, e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Start index rebuild as a background job (RocksDB only).
#[cfg(feature = "storage-rocksdb")]
pub async fn start_rebuild_indexes(
    State(state): State<ManagementState<raisin_rocksdb::RocksDBStorage>>,
    Path(tenant): Path<String>,
    Json(req): Json<RebuildRequest>,
) -> Result<Json<ApiResponse<String>>, StatusCode> {
    use raisin_storage::jobs::{global_registry, JobType};

    let index_type = match parse_index_type(&req.index_type) {
        Ok(t) => t,
        Err(msg) => return Ok(Json(ApiResponse::err(msg))),
    };

    tracing::info!(
        "Starting async index rebuild for tenant '{}', type: {:?}",
        tenant,
        index_type
    );

    let job_id = match global_registry()
        .register_job(
            JobType::IndexRebuild,
            Some(tenant.clone()),
            None,
            None,
            None,
        )
        .await
    {
        Ok(id) => id,
        Err(e) => {
            tracing::error!("Failed to register index rebuild job: {}", e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    let storage = state.storage.clone();
    let job_id_clone = job_id.clone();
    tokio::spawn(async move {
        // TODO: Re-implement when background jobs are available
        if false {
            let e: anyhow::Error = anyhow::anyhow!("Not implemented");
            tracing::error!("Index rebuild job failed: {}", e);
        }
    });

    Ok(Json(ApiResponse::ok(job_id.0)))
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn start_rebuild_indexes<S>(
    State(_state): State<ManagementState<S>>,
    Path(_tenant): Path<String>,
    Json(_req): Json<RebuildRequest>,
) -> Result<Json<ApiResponse<String>>, StatusCode>
where
    S: ManagementOps + BackgroundJobs + Send + Sync,
{
    tracing::error!("Async index rebuild jobs are only supported with RocksDB storage");
    Err(StatusCode::NOT_IMPLEMENTED)
}

// ---------------------------------------------------------------------------
// Orphan cleanup
// ---------------------------------------------------------------------------

/// Synchronous orphan cleanup for a tenant.
pub async fn cleanup_orphans<S>(
    State(state): State<ManagementState<S>>,
    Path(tenant): Path<String>,
) -> Result<Json<ApiResponse<u32>>, StatusCode>
where
    S: ManagementOps + Send + Sync,
{
    match state.storage.cleanup_orphans(&tenant).await {
        Ok(count) => Ok(Json(ApiResponse::ok(count))),
        Err(e) => {
            tracing::error!("Failed to cleanup orphans for tenant {}: {}", tenant, e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Start orphan cleanup as a background job (RocksDB only).
#[cfg(feature = "storage-rocksdb")]
pub async fn start_cleanup_orphans(
    State(state): State<ManagementState<raisin_rocksdb::RocksDBStorage>>,
    Path(tenant): Path<String>,
) -> Result<Json<ApiResponse<String>>, StatusCode> {
    use raisin_storage::jobs::{global_registry, JobType};

    tracing::info!("Starting async orphan cleanup for tenant '{}'", tenant);

    let job_id = match global_registry()
        .register_job(
            JobType::OrphanCleanup,
            Some(tenant.clone()),
            None,
            None,
            None,
        )
        .await
    {
        Ok(id) => id,
        Err(e) => {
            tracing::error!("Failed to register orphan cleanup job: {}", e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    let storage = state.storage.clone();
    let job_id_clone = job_id.clone();
    tokio::spawn(async move {
        // TODO: Re-implement when background jobs are available
        if false {
            let e: anyhow::Error = anyhow::anyhow!("Not implemented");
            tracing::error!("Orphan cleanup job failed: {}", e);
        }
    });

    Ok(Json(ApiResponse::ok(job_id.0)))
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn start_cleanup_orphans<S>(
    State(_state): State<ManagementState<S>>,
    Path(_tenant): Path<String>,
) -> Result<Json<ApiResponse<String>>, StatusCode>
where
    S: ManagementOps + BackgroundJobs + Send + Sync,
{
    tracing::error!("Async orphan cleanup jobs are only supported with RocksDB storage");
    Err(StatusCode::NOT_IMPLEMENTED)
}

// ---------------------------------------------------------------------------
// Property index orphan cleanup
// ---------------------------------------------------------------------------

/// Cleanup orphaned property index entries (RocksDB-specific).
///
/// Scans property indexes and removes entries pointing to nodes that no longer
/// exist. Fixes issues where direct CRUD operations failed between atomic batch
/// write and revision indexing.
#[cfg(feature = "storage-rocksdb")]
pub async fn cleanup_property_index_orphans(
    State(state): State<ManagementState<raisin_rocksdb::RocksDBStorage>>,
    Path(tenant): Path<String>,
) -> Result<Json<ApiResponse<serde_json::Value>>, StatusCode> {
    use raisin_rocksdb::management::async_indexing::cleanup_orphaned_property_indexes;

    tracing::info!(
        "Starting property index orphan cleanup for tenant '{}'",
        tenant
    );

    let repos = match raisin_rocksdb::management::list_repositories(&state.storage, &tenant).await {
        Ok(repos) => repos,
        Err(e) => {
            tracing::error!("Failed to list repositories for tenant {}: {}", tenant, e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    let mut total_stats = serde_json::json!({
        "entries_scanned": 0,
        "orphaned_found": 0,
        "orphaned_deleted": 0,
        "errors": 0,
        "duration_ms": 0,
        "workspaces_processed": 0
    });

    for repo_id in repos {
        let branches = match raisin_rocksdb::management::list_branches(
            &state.storage,
            &tenant,
            &repo_id,
        )
        .await
        {
            Ok(branches) => branches,
            Err(e) => {
                tracing::warn!("Failed to list branches for {}/{}: {}", tenant, repo_id, e);
                continue;
            }
        };

        for branch in branches {
            let workspaces = match raisin_rocksdb::management::list_workspaces(
                &state.storage,
                &tenant,
                &repo_id,
            )
            .await
            {
                Ok(workspaces) => workspaces,
                Err(e) => {
                    tracing::warn!(
                        "Failed to list workspaces for {}/{}: {}",
                        tenant,
                        repo_id,
                        e
                    );
                    continue;
                }
            };

            for workspace in workspaces {
                match cleanup_orphaned_property_indexes(
                    &state.storage,
                    &tenant,
                    &repo_id,
                    &branch,
                    &workspace,
                )
                .await
                {
                    Ok(stats) => {
                        accumulate_stats(&mut total_stats, &stats);
                    }
                    Err(e) => {
                        tracing::error!(
                            "Failed to cleanup property indexes for {}/{}/{}/{}: {}",
                            tenant,
                            repo_id,
                            branch,
                            workspace,
                            e
                        );
                        total_stats["errors"] =
                            serde_json::json!(total_stats["errors"].as_u64().unwrap_or(0) + 1);
                    }
                }
            }
        }
    }

    tracing::info!(
        "Property index orphan cleanup complete for tenant '{}': {:?}",
        tenant,
        total_stats
    );

    Ok(Json(ApiResponse::ok(total_stats)))
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn cleanup_property_index_orphans<S>(
    State(_state): State<ManagementState<S>>,
    Path(_tenant): Path<String>,
) -> Result<Json<ApiResponse<serde_json::Value>>, StatusCode>
where
    S: ManagementOps + Send + Sync,
{
    tracing::error!("Property index orphan cleanup is only supported with RocksDB storage");
    Err(StatusCode::NOT_IMPLEMENTED)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse an index type string into the enum variant.
fn parse_index_type(s: &str) -> Result<IndexType, String> {
    match s {
        "property" => Ok(IndexType::Property),
        "reference" => Ok(IndexType::Reference),
        "child_order" => Ok(IndexType::ChildOrder),
        "all" => Ok(IndexType::All),
        _ => Err(format!("Invalid index type: {}", s)),
    }
}

/// Accumulate property index cleanup stats into the running totals.
#[cfg(feature = "storage-rocksdb")]
fn accumulate_stats(
    total: &mut serde_json::Value,
    stats: &raisin_rocksdb::management::async_indexing::OrphanedIndexCleanupStats,
) {
    total["entries_scanned"] = serde_json::json!(
        total["entries_scanned"].as_u64().unwrap_or(0) + stats.entries_scanned as u64
    );
    total["orphaned_found"] = serde_json::json!(
        total["orphaned_found"].as_u64().unwrap_or(0) + stats.orphaned_found as u64
    );
    total["orphaned_deleted"] = serde_json::json!(
        total["orphaned_deleted"].as_u64().unwrap_or(0) + stats.orphaned_deleted as u64
    );
    total["errors"] =
        serde_json::json!(total["errors"].as_u64().unwrap_or(0) + stats.errors as u64);
    total["duration_ms"] =
        serde_json::json!(total["duration_ms"].as_u64().unwrap_or(0) + stats.duration_ms);
    total["workspaces_processed"] =
        serde_json::json!(total["workspaces_processed"].as_u64().unwrap_or(0) + 1);
}
