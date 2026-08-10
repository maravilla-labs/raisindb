//! Compaction and metrics handlers for management API.

use axum::{extract::State, http::StatusCode, response::Json};
use raisin_storage::{BackgroundJobs, ManagementOps};
use raisin_transport_http::middleware::ScopedTenant;

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
    ScopedTenant(tenant): ScopedTenant,
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
    ScopedTenant(tenant): ScopedTenant,
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
        // tenant unknown for global compaction op; phase G will tighten
        .register_job(JobType::Compaction, String::new(), None, None, None)
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

/// Start compaction for a specific tenant as a background job (RocksDB only).
///
/// Returns immediately with a job ID that can be monitored via SSE. The job is
/// registered against the supplied tenant so it is visible to that tenant's
/// scoped job queries.
#[cfg(feature = "storage-rocksdb")]
pub async fn start_tenant_compaction(
    State(state): State<ManagementState<raisin_rocksdb::RocksDBStorage>>,
    ScopedTenant(tenant): ScopedTenant,
) -> Result<Json<ApiResponse<String>>, StatusCode> {
    use raisin_storage::jobs::{global_registry, JobType};

    tracing::info!(tenant = %tenant, "Starting async compaction for tenant");

    let job_id = match global_registry()
        .register_job(JobType::Compaction, tenant.clone(), None, None, None)
        .await
    {
        Ok(id) => id,
        Err(e) => {
            tracing::error!(
                tenant = %tenant,
                "Failed to register tenant compaction job: {}",
                e
            );
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    let storage = state.storage.clone();
    let job_id_clone = job_id.clone();
    let tenant_for_task = tenant.clone();
    tokio::spawn(async move {
        match storage.compact(Some(&tenant_for_task)).await {
            Ok(stats) => {
                tracing::info!(
                    tenant = %tenant_for_task,
                    job_id = %job_id_clone,
                    "Tenant compaction completed: {:?}",
                    stats
                );
            }
            Err(e) => {
                tracing::error!(
                    tenant = %tenant_for_task,
                    job_id = %job_id_clone,
                    "Tenant compaction job failed: {}",
                    e
                );
            }
        }
    });

    Ok(Json(ApiResponse::ok(job_id.0)))
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn start_tenant_compaction<S>(
    State(_state): State<ManagementState<S>>,
    ScopedTenant(_tenant): ScopedTenant,
) -> Result<Json<ApiResponse<String>>, StatusCode>
where
    S: ManagementOps + BackgroundJobs + Send + Sync,
{
    tracing::error!("Async tenant compaction jobs are only supported with RocksDB storage");
    Err(StatusCode::NOT_IMPLEMENTED)
}

// ---------------------------------------------------------------------------
// Memory diagnostics
// ---------------------------------------------------------------------------

/// Read-only memory sample: allocator counters, per-column-family RocksDB
/// properties, and the sizes of the long-lived in-process collections.
///
/// Poll this; a single reading says almost nothing, and the slope across a few
/// hours identifies the mechanism. See `crate::diagnostics` for how to read the
/// payload.
///
/// Cheap by construction — every field is a counter read or a `len()`, never a
/// scan — so it is safe on a monitoring interval.
#[cfg(feature = "storage-rocksdb")]
pub async fn get_memory_diagnostics(
    State(state): State<ManagementState<raisin_rocksdb::RocksDBStorage>>,
) -> Json<ApiResponse<crate::diagnostics::MemoryDiagnostics>> {
    Json(ApiResponse::ok(
        crate::diagnostics::sample(&state.storage).await,
    ))
}

/// jemalloc's full human-readable statistics dump.
///
/// This is what distinguishes "retained but free" from "genuinely held" —
/// per-arena dirty/muzzy page counts and bin fragmentation. Returns plain text.
/// More expensive than the sample above (it walks every arena and briefly takes
/// allocator locks), so call it deliberately, not on a poll.
///
/// Unavailable off Linux, where jemalloc's `stats` feature is not compiled in.
pub async fn get_malloc_stats() -> Result<String, (StatusCode, String)> {
    crate::diagnostics::allocator::malloc_stats_text()
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e))
}
