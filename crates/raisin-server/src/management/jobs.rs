//! Job management handlers for the management API.
//!
//! Provides endpoints for listing, querying, deleting, cancelling, and
//! scheduling background jobs, as well as queue maintenance operations.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use raisin_storage::BackgroundJobs;

use super::types::{
    ApiResponse, BatchDeleteJobsRequest, BatchDeleteJobsResponse, ForceFailStuckRequest,
    ForceFailStuckResponse, PurgeResponse, ScheduleIntegrityRequest,
};
use super::ManagementState;

// ---------------------------------------------------------------------------
// CRUD operations
// ---------------------------------------------------------------------------

/// List all background jobs.
pub async fn list_jobs<S>(
    State(state): State<ManagementState<S>>,
) -> Result<Json<ApiResponse<Vec<raisin_storage::JobInfo>>>, StatusCode>
where
    S: BackgroundJobs + Send + Sync,
{
    match state.storage.list_jobs().await {
        Ok(jobs) => Ok(Json(ApiResponse::ok(jobs))),
        Err(e) => {
            tracing::error!("Failed to list jobs: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Get the status of a specific job.
pub async fn get_job_status<S>(
    State(state): State<ManagementState<S>>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<raisin_storage::JobStatus>>, StatusCode>
where
    S: BackgroundJobs + Send + Sync,
{
    let job_id = raisin_storage::JobId::from_string(id);
    match state.storage.get_job_status(&job_id).await {
        Ok(status) => Ok(Json(ApiResponse::ok(status))),
        Err(e) => {
            tracing::error!("Failed to get job status: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Get detailed info for a specific job.
pub async fn get_job_info<S>(
    State(state): State<ManagementState<S>>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<raisin_storage::JobInfo>>, StatusCode>
where
    S: BackgroundJobs + Send + Sync,
{
    let job_id = raisin_storage::JobId::from_string(id);
    match state.storage.get_job_info(&job_id).await {
        Ok(info) => Ok(Json(ApiResponse::ok(info))),
        Err(e) => {
            tracing::error!("Failed to get job info: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Delete a specific job.
pub async fn delete_job<S>(
    State(state): State<ManagementState<S>>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, StatusCode>
where
    S: BackgroundJobs + Send + Sync,
{
    let job_id = raisin_storage::JobId::from_string(id);
    match state.storage.delete_job(&job_id).await {
        Ok(()) => Ok(Json(ApiResponse::ok(()))),
        Err(e) => {
            tracing::error!("Failed to delete job: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Batch-delete multiple jobs.
pub async fn batch_delete_jobs<S>(
    State(state): State<ManagementState<S>>,
    Json(req): Json<BatchDeleteJobsRequest>,
) -> Json<ApiResponse<BatchDeleteJobsResponse>>
where
    S: BackgroundJobs + Send + Sync,
{
    let job_ids: Vec<raisin_storage::JobId> = req
        .job_ids
        .into_iter()
        .map(raisin_storage::JobId::from_string)
        .collect();

    let (deleted, skipped) = state.storage.delete_jobs_batch(&job_ids).await;

    tracing::info!(
        deleted = deleted,
        skipped = skipped,
        total = deleted + skipped,
        "Batch deleted jobs"
    );

    Json(ApiResponse::ok(BatchDeleteJobsResponse {
        deleted,
        skipped,
    }))
}

/// Cancel a running job.
pub async fn cancel_job<S>(
    State(state): State<ManagementState<S>>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, StatusCode>
where
    S: BackgroundJobs + Send + Sync,
{
    let job_id = raisin_storage::JobId::from_string(id);
    match state.storage.cancel_job(&job_id).await {
        Ok(()) => Ok(Json(ApiResponse::ok(()))),
        Err(e) => {
            tracing::error!("Failed to cancel job: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

// ---------------------------------------------------------------------------
// Scheduling
// ---------------------------------------------------------------------------

/// Schedule a recurring integrity scan for a tenant.
pub async fn schedule_integrity_scan<S>(
    State(state): State<ManagementState<S>>,
    Json(req): Json<ScheduleIntegrityRequest>,
) -> Result<Json<ApiResponse<String>>, StatusCode>
where
    S: BackgroundJobs + Send + Sync,
{
    let duration = std::time::Duration::from_secs(req.interval_minutes * 60);
    match state.storage.schedule_integrity_scan(&req.tenant, duration) {
        Ok(job_id) => Ok(Json(ApiResponse::ok(job_id.0))),
        Err(e) => {
            tracing::error!(
                "Failed to schedule integrity scan for tenant {}: {}",
                req.tenant,
                e
            );
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

// ---------------------------------------------------------------------------
// Queue maintenance
// ---------------------------------------------------------------------------

/// Get job queue statistics.
pub async fn get_job_queue_stats<S>(
    State(state): State<ManagementState<S>>,
) -> Result<Json<ApiResponse<raisin_storage::JobQueueStats>>, StatusCode>
where
    S: BackgroundJobs + Send + Sync,
{
    match state.storage.get_job_queue_stats().await {
        Ok(stats) => Ok(Json(ApiResponse::ok(stats))),
        Err(e) => {
            tracing::error!("Failed to get job queue stats: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Purge all jobs from persistent storage (admin action).
pub async fn purge_all_jobs<S>(
    State(state): State<ManagementState<S>>,
) -> Result<Json<ApiResponse<PurgeResponse>>, StatusCode>
where
    S: BackgroundJobs + Send + Sync,
{
    tracing::warn!("Purging ALL jobs from persistent storage (admin action)");
    match state.storage.purge_all_jobs().await {
        Ok(purged) => {
            tracing::info!(purged = purged, "Successfully purged all jobs");
            Ok(Json(ApiResponse::ok(PurgeResponse { purged })))
        }
        Err(e) => {
            tracing::error!("Failed to purge all jobs: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Purge orphaned (undeserializable) jobs from persistent storage.
pub async fn purge_orphaned_jobs<S>(
    State(state): State<ManagementState<S>>,
) -> Result<Json<ApiResponse<PurgeResponse>>, StatusCode>
where
    S: BackgroundJobs + Send + Sync,
{
    tracing::info!("Purging orphaned (undeserializable) jobs from persistent storage");
    match state.storage.purge_orphaned_jobs().await {
        Ok(purged) => {
            tracing::info!(purged = purged, "Successfully purged orphaned jobs");
            Ok(Json(ApiResponse::ok(PurgeResponse { purged })))
        }
        Err(e) => {
            tracing::error!("Failed to purge orphaned jobs: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Force-fail jobs stuck in running state beyond a threshold.
pub async fn force_fail_stuck_jobs<S>(
    State(state): State<ManagementState<S>>,
    Json(req): Json<ForceFailStuckRequest>,
) -> Result<Json<ApiResponse<ForceFailStuckResponse>>, StatusCode>
where
    S: BackgroundJobs + Send + Sync,
{
    tracing::warn!(
        stuck_minutes = req.stuck_minutes,
        "Force-failing stuck jobs (admin action)"
    );
    match state.storage.force_fail_stuck_jobs(req.stuck_minutes).await {
        Ok((failed_count, job_ids)) => {
            tracing::info!(
                failed_count = failed_count,
                "Successfully force-failed stuck jobs"
            );
            Ok(Json(ApiResponse::ok(ForceFailStuckResponse {
                failed_count,
                job_ids,
            })))
        }
        Err(e) => {
            tracing::error!("Failed to force-fail stuck jobs: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}
