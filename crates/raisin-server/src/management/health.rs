//! Health check handlers for management API.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    Extension,
};
use raisin_storage::ManagementOps;

use super::types::ApiResponse;
use super::ManagementState;

/// Overall storage health check.
pub async fn get_health<S>(
    State(state): State<ManagementState<S>>,
) -> Result<Json<ApiResponse<raisin_storage::HealthStatus>>, StatusCode>
where
    S: ManagementOps + Send + Sync,
{
    match state.storage.get_health(None).await {
        Ok(health) => Ok(Json(ApiResponse::ok(health))),
        Err(e) => {
            tracing::error!("Failed to get health status: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Per-tenant health check.
pub async fn get_tenant_health<S>(
    State(state): State<ManagementState<S>>,
    Path(tenant): Path<String>,
) -> Result<Json<ApiResponse<raisin_storage::HealthStatus>>, StatusCode>
where
    S: ManagementOps + Send + Sync,
{
    match state.storage.get_health(Some(&tenant)).await {
        Ok(health) => Ok(Json(ApiResponse::ok(health))),
        Err(e) => {
            tracing::error!("Failed to get health status for tenant {}: {}", tenant, e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Replication metrics endpoint (RocksDB only).
#[cfg(feature = "storage-rocksdb")]
pub async fn replication_metrics_handler(
    Extension(monitoring): Extension<Arc<raisin_rocksdb::monitoring::MonitoringService>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    match monitoring.export_json_value().await {
        Ok(json) => Ok(Json(json)),
        Err(e) => {
            tracing::error!("Failed to export metrics: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Enhanced health check including monitoring status (RocksDB only).
#[cfg(feature = "storage-rocksdb")]
pub async fn get_health_with_monitoring(
    State(state): State<ManagementState<raisin_rocksdb::RocksDBStorage>>,
    Extension(monitoring): Extension<Arc<raisin_rocksdb::monitoring::MonitoringService>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // Get storage health
    let storage_health = match state.storage.get_health(None).await {
        Ok(health) => health,
        Err(e) => {
            tracing::error!("Failed to get health status: {}", e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    // Get monitoring health
    let monitoring_healthy = monitoring.is_healthy().await;
    let overall_status = if monitoring_healthy {
        "healthy"
    } else {
        "degraded"
    };

    Ok(Json(serde_json::json!({
        "status": overall_status,
        "storage": storage_health,
        "monitoring": {
            "healthy": monitoring_healthy
        },
        "timestamp": chrono::Utc::now().to_rfc3339()
    })))
}

#[cfg(feature = "storage-rocksdb")]
use std::sync::Arc;
