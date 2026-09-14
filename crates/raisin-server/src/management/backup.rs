//! Backup and repair handlers for management API.
//!
//! Provides synchronous backup endpoints (generic) and async background-job
//! variants (RocksDB-specific) for backup and repair operations.

use axum::{extract::State, http::StatusCode, response::Json};
use raisin_storage::{BackgroundJobs, ManagementOps};
use raisin_transport_http::middleware::ScopedTenant;

use super::types::{ApiResponse, BackupRequest, RepairRequest};
use super::ManagementState;

// ---------------------------------------------------------------------------
// Tenant backup
// ---------------------------------------------------------------------------

/// Backup a single tenant to the given path.
pub async fn backup_tenant<S>(
    State(state): State<ManagementState<S>>,
    ScopedTenant(tenant): ScopedTenant,
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
    State(_state): State<ManagementState<raisin_rocksdb::RocksDBStorage>>,
    Json(req): Json<BackupRequest>,
) -> Result<Json<ApiResponse<String>>, StatusCode> {
    // This never ran: it registered a job on a registry no worker reads and
    // returned its id. Say so instead of handing out a job that never
    // completes. The synchronous `/management/admin/backup/all` (what the
    // nightly backup timer calls) is unaffected.
    tracing::warn!(path = %req.path, "async full backup is not implemented; use /management/admin/backup/all");
    Err(StatusCode::NOT_IMPLEMENTED)
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
    ScopedTenant(tenant): ScopedTenant,
    Json(req): Json<RepairRequest>,
) -> Result<Json<ApiResponse<String>>, StatusCode> {
    use raisin_storage::jobs::JobType;

    // The job repairs what a fresh server-side scan finds, not this list:
    // accepted for compatibility, never trusted.
    tracing::info!(
        tenant = %tenant,
        client_issues = req.issues.len(),
        "Starting repair (issues are re-scanned server-side)"
    );

    let job_id = super::queue_job::queue_maintenance_job(
        &state.storage,
        JobType::Repair,
        &tenant,
        std::collections::HashMap::new(),
    )
    .await?;
    tracing::info!(job_id = %job_id, "repair queued");

    Ok(Json(ApiResponse::ok(job_id.0)))
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn start_repair<S>(
    State(_state): State<ManagementState<S>>,
    ScopedTenant(_tenant): ScopedTenant,
    Json(_req): Json<RepairRequest>,
) -> Result<Json<ApiResponse<String>>, StatusCode>
where
    S: ManagementOps + BackgroundJobs + Send + Sync,
{
    tracing::error!("Async repair jobs are only supported with RocksDB storage");
    Err(StatusCode::NOT_IMPLEMENTED)
}
