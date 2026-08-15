//! Job management handlers for the management API.
//!
//! Provides endpoints for listing, querying, deleting, cancelling, and
//! scheduling background jobs, as well as queue maintenance operations.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    Extension,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use raisin_storage::BackgroundJobs;
use raisin_transport_http::middleware::TenantInfo;

use super::types::{
    ApiResponse, BatchDeleteJobsRequest, BatchDeleteJobsResponse, ForceFailStuckRequest,
    ForceFailStuckResponse, PurgeResponse, ScheduleIntegrityRequest,
};
use super::ManagementState;

/// Map a storage error from a job point-lookup to an HTTP status code.
///
/// `raisin_error::Error::NotFound` → `404 Not Found` (covers both
/// genuinely missing job IDs and cross-tenant access attempts — the
/// storage layer returns `NotFound` in both cases so we never leak the
/// existence of jobs belonging to other tenants). Everything else maps to
/// `500 Internal Server Error`.
fn job_lookup_status(err: &raisin_error::Error) -> StatusCode {
    match err {
        raisin_error::Error::NotFound(_) => StatusCode::NOT_FOUND,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// One explicitly requested, bounded history-index backfill batch.
#[derive(Debug, serde::Deserialize)]
pub struct HistoryBackfillRequest {
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, serde::Serialize)]
pub struct HistoryBackfillResponse {
    pub indexed: usize,
    pub next_cursor: Option<String>,
}

/// Backfill a capped page of the durable job-history index.
///
/// This endpoint is RocksDB-only because it deliberately scans persistent
/// metadata. It is never used by the jobs page, SSE stream, or polling loop.
#[cfg(feature = "storage-rocksdb")]
pub async fn backfill_job_history_index(
    State(state): State<ManagementState<raisin_rocksdb::RocksDBStorage>>,
    Extension(tenant_info): Extension<TenantInfo>,
    Json(req): Json<HistoryBackfillRequest>,
) -> Result<Json<ApiResponse<HistoryBackfillResponse>>, StatusCode> {
    let cursor = match req.cursor {
        Some(cursor) => URL_SAFE_NO_PAD
            .decode(cursor)
            .map_err(|_| StatusCode::BAD_REQUEST)?,
        None => Vec::new(),
    };
    let limit = req.limit.unwrap_or(500).clamp(1, 1_000);
    let store = state.storage.job_metadata_store().clone();
    let tenant = tenant_info.tenant_id.clone();
    let tenant_for_task = tenant.clone();
    let result = tokio::task::spawn_blocking(move || {
        store.backfill_history_index_page(
            &tenant_for_task,
            limit,
            (!cursor.is_empty()).then_some(cursor.as_slice()),
        )
    })
    .await
    .map_err(|error| {
        tracing::error!(%error, "Job history backfill task panicked");
        StatusCode::INTERNAL_SERVER_ERROR
    })?
    .map_err(|error| {
        tracing::error!(%error, tenant = %tenant, "Job history backfill failed");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(ApiResponse::ok(HistoryBackfillResponse {
        indexed: result.0,
        next_cursor: result.1.map(|cursor| URL_SAFE_NO_PAD.encode(cursor)),
    })))
}

// ---------------------------------------------------------------------------
// CRUD operations
// ---------------------------------------------------------------------------

/// Query filters for listing jobs.
#[derive(Debug, serde::Deserialize, Default)]
pub struct ListJobsQuery {
    /// Restrict to jobs whose persisted context records this repository.
    /// Jobs without a recorded scope are always included.
    #[serde(default)]
    pub repo: Option<String>,
}

/// List all background jobs for the request's tenant, each with its
/// execution scope (repo/branch/workspace) when known. Pass `?repo=` to
/// filter to one repository.
pub async fn list_jobs<S>(
    State(state): State<ManagementState<S>>,
    Extension(tenant_info): Extension<TenantInfo>,
    Query(query): Query<ListJobsQuery>,
) -> Result<Json<ApiResponse<Vec<raisin_storage::ScopedJobInfo>>>, StatusCode>
where
    S: BackgroundJobs + Send + Sync,
{
    match state
        .storage
        .list_jobs_with_scope(&tenant_info.tenant_id, query.repo.as_deref())
        .await
    {
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
    Extension(tenant_info): Extension<TenantInfo>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<raisin_storage::JobStatus>>, StatusCode>
where
    S: BackgroundJobs + Send + Sync,
{
    let job_id = raisin_storage::JobId::from_string(id);
    match state
        .storage
        .get_job_status(&tenant_info.tenant_id, &job_id)
        .await
    {
        Ok(status) => Ok(Json(ApiResponse::ok(status))),
        Err(e) => {
            // 404 on tenant mismatch / missing job (no info leak); 500 otherwise.
            let code = job_lookup_status(&e);
            if code == StatusCode::NOT_FOUND {
                tracing::debug!(
                    tenant = %tenant_info.tenant_id,
                    job_id = %job_id,
                    "Job status lookup: not found (or different tenant)"
                );
            } else {
                tracing::error!("Failed to get job status: {}", e);
            }
            Err(code)
        }
    }
}

/// Get detailed info for a specific job.
pub async fn get_job_info<S>(
    State(state): State<ManagementState<S>>,
    Extension(tenant_info): Extension<TenantInfo>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<raisin_storage::JobInfo>>, StatusCode>
where
    S: BackgroundJobs + Send + Sync,
{
    let job_id = raisin_storage::JobId::from_string(id);
    match state
        .storage
        .get_job_info(&tenant_info.tenant_id, &job_id)
        .await
    {
        Ok(info) => Ok(Json(ApiResponse::ok(info))),
        Err(e) => {
            let code = job_lookup_status(&e);
            if code == StatusCode::NOT_FOUND {
                tracing::debug!(
                    tenant = %tenant_info.tenant_id,
                    job_id = %job_id,
                    "Job info lookup: not found (or different tenant)"
                );
            } else {
                tracing::error!("Failed to get job info: {}", e);
            }
            Err(code)
        }
    }
}

/// Delete a specific job.
pub async fn delete_job<S>(
    State(state): State<ManagementState<S>>,
    Extension(tenant_info): Extension<TenantInfo>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, StatusCode>
where
    S: BackgroundJobs + Send + Sync,
{
    let job_id = raisin_storage::JobId::from_string(id);
    match state
        .storage
        .delete_job(&tenant_info.tenant_id, &job_id)
        .await
    {
        Ok(()) => Ok(Json(ApiResponse::ok(()))),
        Err(e) => {
            let code = job_lookup_status(&e);
            if code == StatusCode::NOT_FOUND {
                tracing::debug!(
                    tenant = %tenant_info.tenant_id,
                    job_id = %job_id,
                    "Job delete: not found (or different tenant)"
                );
            } else {
                tracing::error!("Failed to delete job: {}", e);
            }
            Err(code)
        }
    }
}

/// Batch-delete multiple jobs.
pub async fn batch_delete_jobs<S>(
    State(state): State<ManagementState<S>>,
    Extension(tenant_info): Extension<TenantInfo>,
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

    let (deleted, skipped) = state
        .storage
        .delete_jobs_batch(&tenant_info.tenant_id, &job_ids)
        .await;

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
    Extension(tenant_info): Extension<TenantInfo>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, StatusCode>
where
    S: BackgroundJobs + Send + Sync,
{
    let job_id = raisin_storage::JobId::from_string(id);
    match state
        .storage
        .cancel_job(&tenant_info.tenant_id, &job_id)
        .await
    {
        Ok(()) => Ok(Json(ApiResponse::ok(()))),
        Err(e) => {
            let code = job_lookup_status(&e);
            if code == StatusCode::NOT_FOUND {
                tracing::debug!(
                    tenant = %tenant_info.tenant_id,
                    job_id = %job_id,
                    "Job cancel: not found (or different tenant)"
                );
            } else {
                tracing::error!("Failed to cancel job: {}", e);
            }
            Err(code)
        }
    }
}

// ---------------------------------------------------------------------------
// Scheduling
// ---------------------------------------------------------------------------

/// Schedule a recurring integrity scan for the request's tenant.
pub async fn schedule_integrity_scan<S>(
    State(state): State<ManagementState<S>>,
    Extension(tenant_info): Extension<TenantInfo>,
    Json(req): Json<ScheduleIntegrityRequest>,
) -> Result<Json<ApiResponse<String>>, StatusCode>
where
    S: BackgroundJobs + Send + Sync,
{
    let duration = std::time::Duration::from_secs(req.interval_minutes * 60);
    match state
        .storage
        .schedule_integrity_scan(&tenant_info.tenant_id, duration)
    {
        Ok(job_id) => Ok(Json(ApiResponse::ok(job_id.0))),
        Err(e) => {
            tracing::error!(
                "Failed to schedule integrity scan for tenant {}: {}",
                tenant_info.tenant_id,
                e
            );
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

// ---------------------------------------------------------------------------
// Queue maintenance
// ---------------------------------------------------------------------------

/// Get job queue statistics for the request's tenant.
pub async fn get_job_queue_stats<S>(
    State(state): State<ManagementState<S>>,
    Extension(tenant_info): Extension<TenantInfo>,
) -> Result<Json<ApiResponse<raisin_storage::JobQueueStats>>, StatusCode>
where
    S: BackgroundJobs + Send + Sync,
{
    match state
        .storage
        .get_job_queue_stats(&tenant_info.tenant_id)
        .await
    {
        Ok(stats) => Ok(Json(ApiResponse::ok(stats))),
        Err(e) => {
            tracing::error!("Failed to get job queue stats: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Purge all jobs from persistent storage for the request's tenant.
pub async fn purge_all_jobs<S>(
    State(state): State<ManagementState<S>>,
    Extension(tenant_info): Extension<TenantInfo>,
) -> Result<Json<ApiResponse<PurgeResponse>>, StatusCode>
where
    S: BackgroundJobs + Send + Sync,
{
    tracing::warn!(
        tenant = %tenant_info.tenant_id,
        "Purging tenant jobs from persistent storage"
    );
    match state.storage.purge_all_jobs(&tenant_info.tenant_id).await {
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
    Extension(tenant_info): Extension<TenantInfo>,
) -> Result<Json<ApiResponse<PurgeResponse>>, StatusCode>
where
    S: BackgroundJobs + Send + Sync,
{
    tracing::info!("Purging orphaned (undeserializable) jobs from persistent storage");
    match state
        .storage
        .purge_orphaned_jobs(&tenant_info.tenant_id)
        .await
    {
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

/// Force-fail jobs stuck in running state beyond a threshold for this tenant.
pub async fn force_fail_stuck_jobs<S>(
    State(state): State<ManagementState<S>>,
    Extension(tenant_info): Extension<TenantInfo>,
    Json(req): Json<ForceFailStuckRequest>,
) -> Result<Json<ApiResponse<ForceFailStuckResponse>>, StatusCode>
where
    S: BackgroundJobs + Send + Sync,
{
    tracing::warn!(
        tenant = %tenant_info.tenant_id,
        stuck_minutes = req.stuck_minutes,
        "Force-failing stuck jobs (admin action)"
    );
    match state
        .storage
        .force_fail_stuck_jobs(&tenant_info.tenant_id, req.stuck_minutes)
        .await
    {
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
