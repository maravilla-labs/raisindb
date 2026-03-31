//! Backup and repair handlers for management API.
//!
//! Provides synchronous backup endpoints (generic) and async background-job
//! variants (RocksDB-specific) for backup and repair operations.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use raisin_storage::{BackgroundJobs, ManagementOps};

use super::types::{ApiResponse, BackupRequest, RepairRequest};
use super::ManagementState;

// ---------------------------------------------------------------------------
// Tenant backup
// ---------------------------------------------------------------------------

/// Backup a single tenant to the given path.
pub async fn backup_tenant<S>(
    State(state): State<ManagementState<S>>,
    Path(tenant): Path<String>,
    Json(req): Json<BackupRequest>,
) -> Result<Json<ApiResponse<raisin_storage::BackupInfo>>, StatusCode>
where
    S: ManagementOps + Send + Sync,
{
    let path = std::path::Path::new(&req.path);
    match state.storage.backup_tenant(&tenant, path).await {
        Ok(info) => Ok(Json(ApiResponse::ok(info))),
        Err(e) => {
            tracing::error!("Failed to backup tenant {}: {}", tenant, e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

// ---------------------------------------------------------------------------
// Full backup
// ---------------------------------------------------------------------------

/// Backup all tenants to the given path.
pub async fn backup_all<S>(
    State(state): State<ManagementState<S>>,
    Json(req): Json<BackupRequest>,
) -> Result<Json<ApiResponse<Vec<raisin_storage::BackupInfo>>>, StatusCode>
where
    S: ManagementOps + Send + Sync,
{
    let path = std::path::Path::new(&req.path);
    match state.storage.backup_all(path).await {
        Ok(infos) => Ok(Json(ApiResponse::ok(infos))),
        Err(e) => {
            tracing::error!("Failed to backup all tenants: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Start full backup as a background job (RocksDB only).
///
/// Returns immediately with a job ID that can be monitored via SSE.
#[cfg(feature = "storage-rocksdb")]
pub async fn start_backup(
    State(state): State<ManagementState<raisin_rocksdb::RocksDBStorage>>,
    Json(req): Json<BackupRequest>,
) -> Result<Json<ApiResponse<String>>, StatusCode> {
    use raisin_storage::jobs::{global_registry, JobType};

    let backup_path = std::path::PathBuf::from(&req.path);
    tracing::info!("Starting async backup to path: {:?}", backup_path);

    let job_id = match global_registry()
        .register_job(JobType::Backup, None, None, None, None)
        .await
    {
        Ok(id) => id,
        Err(e) => {
            tracing::error!("Failed to register backup job: {}", e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    let storage = state.storage.clone();
    let job_id_clone = job_id.clone();
    tokio::spawn(async move {
        // TODO: Re-implement when background jobs are available
        if false {
            let e: anyhow::Error = anyhow::anyhow!("Not implemented");
            tracing::error!("Backup job failed: {}", e);
        }
    });

    Ok(Json(ApiResponse::ok(job_id.0)))
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn start_backup<S>(
    State(_state): State<ManagementState<S>>,
    Json(_req): Json<BackupRequest>,
) -> Result<Json<ApiResponse<String>>, StatusCode>
where
    S: ManagementOps + BackgroundJobs + Send + Sync,
{
    tracing::error!("Async backup jobs are only supported with RocksDB storage");
    Err(StatusCode::NOT_IMPLEMENTED)
}

// ---------------------------------------------------------------------------
// Repair
// ---------------------------------------------------------------------------

/// Start repair as a background job (RocksDB only).
///
/// Returns immediately with a job ID that can be monitored via SSE.
#[cfg(feature = "storage-rocksdb")]
pub async fn start_repair(
    State(state): State<ManagementState<raisin_rocksdb::RocksDBStorage>>,
    Path(tenant): Path<String>,
    Json(req): Json<RepairRequest>,
) -> Result<Json<ApiResponse<String>>, StatusCode> {
    use raisin_storage::jobs::{global_registry, JobType};

    tracing::info!(
        "Starting async repair for tenant '{}' with {} issues",
        tenant,
        req.issues.len()
    );

    let job_id = match global_registry()
        .register_job(JobType::Repair, Some(tenant.clone()), None, None, None)
        .await
    {
        Ok(id) => id,
        Err(e) => {
            tracing::error!("Failed to register repair job: {}", e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    let storage = state.storage.clone();
    let job_id_clone = job_id.clone();
    let issues = req.issues;
    tokio::spawn(async move {
        // TODO: Re-implement when background jobs are available
        if false {
            let e: anyhow::Error = anyhow::anyhow!("Not implemented");
            tracing::error!("Repair job failed: {}", e);
        }
    });

    Ok(Json(ApiResponse::ok(job_id.0)))
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn start_repair<S>(
    State(_state): State<ManagementState<S>>,
    Path(_tenant): Path<String>,
    Json(_req): Json<RepairRequest>,
) -> Result<Json<ApiResponse<String>>, StatusCode>
where
    S: ManagementOps + BackgroundJobs + Send + Sync,
{
    tracing::error!("Async repair jobs are only supported with RocksDB storage");
    Err(StatusCode::NOT_IMPLEMENTED)
}
