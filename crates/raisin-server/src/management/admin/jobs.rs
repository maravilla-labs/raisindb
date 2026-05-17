//! Superadmin handlers for cross-tenant job operations.

use axum::{extract::State, http::StatusCode, response::Json};
use raisin_storage::BackgroundJobsInternal;
use serde::Deserialize;

use crate::management::{
    types::{ApiResponse, ForceFailStuckRequest, ForceFailStuckResponse, PurgeResponse},
    ManagementState,
};

/// List jobs across **all** tenants.
pub async fn list_all_jobs(
    State(state): State<ManagementState<raisin_rocksdb::RocksDBStorage>>,
) -> Result<Json<ApiResponse<Vec<raisin_storage::JobInfo>>>, StatusCode> {
    match state.storage.list_all_jobs().await {
        Ok(jobs) => Ok(Json(ApiResponse::ok(jobs))),
        Err(e) => {
            tracing::error!(error = %e, "Superadmin: failed to list all jobs");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Purge **all** jobs across **all** tenants.
pub async fn purge_all_global(
    State(state): State<ManagementState<raisin_rocksdb::RocksDBStorage>>,
) -> Result<Json<ApiResponse<PurgeResponse>>, StatusCode> {
    tracing::warn!("Superadmin: purging all jobs across all tenants");
    match state.storage.purge_all_jobs_global().await {
        Ok(purged) => Ok(Json(ApiResponse::ok(PurgeResponse { purged }))),
        Err(e) => {
            tracing::error!(error = %e, "Superadmin: purge_all_global failed");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Body parameter for the global force-fail endpoint (reuses request shape).
#[derive(Debug, Deserialize)]
pub struct ForceFailStuckGlobalRequest {
    pub stuck_minutes: u64,
}

impl From<ForceFailStuckGlobalRequest> for ForceFailStuckRequest {
    fn from(value: ForceFailStuckGlobalRequest) -> Self {
        ForceFailStuckRequest {
            stuck_minutes: value.stuck_minutes,
        }
    }
}

/// Force-fail stuck jobs across **all** tenants.
pub async fn force_fail_stuck_global(
    State(state): State<ManagementState<raisin_rocksdb::RocksDBStorage>>,
    Json(req): Json<ForceFailStuckRequest>,
) -> Result<Json<ApiResponse<ForceFailStuckResponse>>, StatusCode> {
    tracing::warn!(
        stuck_minutes = req.stuck_minutes,
        "Superadmin: force-failing stuck jobs across all tenants"
    );
    match state
        .storage
        .force_fail_stuck_jobs_global(req.stuck_minutes)
        .await
    {
        Ok((failed_count, job_ids)) => Ok(Json(ApiResponse::ok(ForceFailStuckResponse {
            failed_count,
            job_ids,
        }))),
        Err(e) => {
            tracing::error!(error = %e, "Superadmin: force-fail global failed");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}
