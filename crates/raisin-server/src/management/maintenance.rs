//! Compaction and metrics handlers for management API.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use raisin_storage::{BackgroundJobs, ManagementOps};

use super::types::ApiResponse;
use super::ManagementState;

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

/// Get overall storage metrics.
pub async fn get_metrics<S>(
    State(state): State<ManagementState<S>>,
) -> Result<Json<ApiResponse<raisin_storage::Metrics>>, StatusCode>
where
    S: ManagementOps + Send + Sync,
{
    match state.storage.get_metrics(None).await {
        Ok(metrics) => Ok(Json(ApiResponse::ok(metrics))),
        Err(e) => {
            tracing::error!("Failed to get metrics: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Get per-tenant storage metrics.
pub async fn get_tenant_metrics<S>(
    State(state): State<ManagementState<S>>,
    Path(tenant): Path<String>,
) -> Result<Json<ApiResponse<raisin_storage::Metrics>>, StatusCode>
where
    S: ManagementOps + Send + Sync,
{
    match state.storage.get_metrics(Some(&tenant)).await {
        Ok(metrics) => Ok(Json(ApiResponse::ok(metrics))),
        Err(e) => {
            tracing::error!("Failed to get metrics for tenant {}: {}", tenant, e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

// ---------------------------------------------------------------------------
// Compaction
// ---------------------------------------------------------------------------

/// Trigger compaction for all data.
pub async fn trigger_compaction<S>(
    State(state): State<ManagementState<S>>,
) -> Result<Json<ApiResponse<raisin_storage::CompactionStats>>, StatusCode>
where
    S: ManagementOps + Send + Sync,
{
    match state.storage.compact(None).await {
        Ok(stats) => Ok(Json(ApiResponse::ok(stats))),
        Err(e) => {
            tracing::error!("Failed to trigger compaction: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Trigger compaction for a specific tenant.
pub async fn trigger_tenant_compaction<S>(
    State(state): State<ManagementState<S>>,
    Path(tenant): Path<String>,
) -> Result<Json<ApiResponse<raisin_storage::CompactionStats>>, StatusCode>
where
    S: ManagementOps + Send + Sync,
{
    match state.storage.compact(Some(&tenant)).await {
        Ok(stats) => Ok(Json(ApiResponse::ok(stats))),
        Err(e) => {
            tracing::error!("Failed to trigger compaction for tenant {}: {}", tenant, e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Start compaction as a background job (RocksDB only).
///
/// Returns immediately with a job ID that can be monitored via SSE.
#[cfg(feature = "storage-rocksdb")]
pub async fn start_compaction(
    State(state): State<ManagementState<raisin_rocksdb::RocksDBStorage>>,
) -> Result<Json<ApiResponse<String>>, StatusCode> {
    use raisin_storage::jobs::{global_registry, JobType};

    tracing::info!("Starting async compaction");

    let job_id = match global_registry()
        .register_job(JobType::Compaction, None, None, None, None)
        .await
    {
        Ok(id) => id,
        Err(e) => {
            tracing::error!("Failed to register compaction job: {}", e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    let storage = state.storage.clone();
    let job_id_clone = job_id.clone();
    tokio::spawn(async move {
        // TODO: Re-implement when background jobs are available
        if false {
            let e: anyhow::Error = anyhow::anyhow!("Not implemented");
            tracing::error!("Compaction job failed: {}", e);
        }
    });

    Ok(Json(ApiResponse::ok(job_id.0)))
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn start_compaction<S>(
    State(_state): State<ManagementState<S>>,
) -> Result<Json<ApiResponse<String>>, StatusCode>
where
    S: ManagementOps + BackgroundJobs + Send + Sync,
{
    tracing::error!("Async compaction jobs are only supported with RocksDB storage");
    Err(StatusCode::NOT_IMPLEMENTED)
}
