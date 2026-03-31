//! Router construction for the management API.
//!
//! Provides two versions of the router:
//! - RocksDB-specific: with concrete storage types, monitoring, auth middlewares
//! - Generic: for other storage backends implementing `ManagementOps + BackgroundJobs`

use axum::{
    routing::{get, post},
    Router,
};
use raisin_storage::{BackgroundJobs, ManagementOps};
use std::sync::Arc;

use super::backup;
use super::health;
use super::integrity;
use super::jobs;
use super::maintenance;
use super::ManagementState;

#[cfg(feature = "storage-rocksdb")]
use super::graph_cache;

/// Create the management API router (RocksDB version with concrete types).
///
/// This router is secured with:
/// - `ensure_tenant_middleware`: Extracts tenant from `x-tenant-id` header
/// - `require_admin_auth_middleware`: Validates admin JWT tokens
#[cfg(feature = "storage-rocksdb")]
pub fn management_router(
    storage: Arc<raisin_rocksdb::RocksDBStorage>,
    monitoring: Arc<raisin_rocksdb::monitoring::MonitoringService>,
    graph_cache_state: Option<Arc<graph_cache::GraphCacheState>>,
    app_state: raisin_transport_http::state::AppState,
) -> Router {
    use axum::{middleware::from_fn_with_state, Extension};
    use raisin_transport_http::middleware::{
        ensure_tenant_middleware, require_admin_auth_middleware,
    };

    // Get data_dir before storage is moved into state
    let data_dir = storage.config().path.to_string_lossy().to_string();
    let state = ManagementState { storage };

    let router = Router::new()
        // Health endpoints (enhanced with monitoring status)
        .route(
            "/management/health",
            get(health::get_health_with_monitoring),
        )
        .route("/management/health/storage", get(health::get_health))
        .route(
            "/management/health/{tenant}",
            get(health::get_tenant_health),
        )
        // Integrity endpoints
        .route(
            "/management/integrity/{tenant}",
            get(integrity::check_integrity),
        )
        .route(
            "/management/integrity/{tenant}/start",
            post(integrity::start_integrity_check),
        )
        .route(
            "/management/integrity/{tenant}/repair/start",
            post(backup::start_repair),
        )
        .route(
            "/management/integrity/{tenant}/verify",
            get(integrity::verify_indexes),
        )
        .route(
            "/management/integrity/{tenant}/verify/start",
            post(integrity::start_verify_indexes),
        )
        .route(
            "/management/integrity/{tenant}/rebuild",
            post(integrity::rebuild_indexes),
        )
        .route(
            "/management/integrity/{tenant}/rebuild/start",
            post(integrity::start_rebuild_indexes),
        )
        .route(
            "/management/integrity/{tenant}/cleanup",
            post(integrity::cleanup_orphans),
        )
        .route(
            "/management/integrity/{tenant}/cleanup/start",
            post(integrity::start_cleanup_orphans),
        )
        .route(
            "/management/integrity/{tenant}/cleanup-property-indexes",
            post(integrity::cleanup_property_index_orphans),
        )
        // Metrics endpoints
        .route("/management/metrics", get(maintenance::get_metrics))
        .route(
            "/management/metrics/{tenant}",
            get(maintenance::get_tenant_metrics),
        )
        // Maintenance endpoints
        .route("/management/compact", post(maintenance::trigger_compaction))
        .route(
            "/management/compact/start",
            post(maintenance::start_compaction),
        )
        .route(
            "/management/compact/{tenant}",
            post(maintenance::trigger_tenant_compaction),
        )
        // Backup endpoints
        .route("/management/backup/{tenant}", post(backup::backup_tenant))
        .route("/management/backup/all", post(backup::backup_all))
        .route("/management/backup/all/start", post(backup::start_backup))
        // Job management endpoints
        .route("/management/jobs", get(jobs::list_jobs))
        .route("/management/jobs/{id}", get(jobs::get_job_status))
        .route("/management/jobs/{id}/info", get(jobs::get_job_info))
        .route(
            "/management/jobs/{id}",
            axum::routing::delete(jobs::delete_job),
        )
        .route("/management/jobs/{id}/cancel", post(jobs::cancel_job))
        .route(
            "/management/jobs/batch-delete",
            post(jobs::batch_delete_jobs),
        )
        .route(
            "/management/jobs/schedule/integrity",
            post(jobs::schedule_integrity_scan),
        )
        // Job queue management endpoints
        .route("/management/jobs/stats", get(jobs::get_job_queue_stats))
        .route("/management/jobs/purge-all", post(jobs::purge_all_jobs))
        .route(
            "/management/jobs/purge-orphaned",
            post(jobs::purge_orphaned_jobs),
        )
        .route(
            "/management/jobs/force-fail-stuck",
            post(jobs::force_fail_stuck_jobs),
        )
        // SSE streaming endpoints for real-time updates
        .route(
            "/management/events/jobs",
            get(crate::sse::job_events_stream_rocksdb),
        )
        .route(
            "/management/events/health",
            get(crate::sse::health_events_stream::<raisin_rocksdb::RocksDBStorage>),
        )
        .route(
            "/management/events/metrics",
            get(crate::sse::metrics_events_stream::<raisin_rocksdb::RocksDBStorage>),
        )
        .route(
            "/management/metrics/replication",
            get(health::replication_metrics_handler),
        )
        // Graph cache management endpoints
        .route(
            "/management/graph-cache/{repo}/status",
            get(graph_cache::get_graph_cache_status),
        )
        .route(
            "/management/graph-cache/{repo}/{config_id}/recompute",
            post(graph_cache::trigger_recompute),
        )
        .route(
            "/management/graph-cache/{repo}/{config_id}/mark-stale",
            post(graph_cache::mark_stale),
        )
        .route(
            "/management/graph-cache/{repo}/stream",
            get(graph_cache::graph_cache_events_stream),
        )
        .with_state(state)
        .layer(Extension(monitoring));

    // Add dependency management routes (uses separate state for data_dir)
    let deps_state = super::dependencies::DepsState { data_dir };
    let deps_router = Router::new()
        .route(
            "/management/dependencies",
            get(super::dependencies::list_dependencies),
        )
        .route(
            "/management/dependencies/{name}/enable",
            post(super::dependencies::enable_dependency),
        )
        .with_state(deps_state);

    let router = router.merge(deps_router);

    // Add graph cache state as Extension if available
    let router = if let Some(gcs) = graph_cache_state {
        router.layer(Extension(gcs))
    } else {
        router
    };

    // Apply security middlewares
    // ensure_tenant runs FIRST (outer), then require_admin (inner)
    // In Axum layers, later layers run first, so add require_admin first
    router
        .layer(from_fn_with_state(
            app_state.clone(),
            require_admin_auth_middleware,
        ))
        .layer(from_fn_with_state(app_state, ensure_tenant_middleware))
}

/// Create the management API router (generic version for other storage backends).
#[cfg(not(feature = "storage-rocksdb"))]
pub fn management_router<S>(storage: Arc<S>) -> Router
where
    S: ManagementOps + BackgroundJobs + Clone + Send + Sync + 'static,
{
    let state = ManagementState { storage };

    Router::new()
        // Health endpoints
        .route("/management/health", get(health::get_health))
        .route(
            "/management/health/{tenant}",
            get(health::get_tenant_health),
        )
        // Integrity endpoints
        .route(
            "/management/integrity/{tenant}",
            get(integrity::check_integrity),
        )
        .route(
            "/management/integrity/{tenant}/start",
            post(integrity::start_integrity_check),
        )
        .route(
            "/management/integrity/{tenant}/repair/start",
            post(backup::start_repair),
        )
        .route(
            "/management/integrity/{tenant}/verify",
            get(integrity::verify_indexes),
        )
        .route(
            "/management/integrity/{tenant}/verify/start",
            post(integrity::start_verify_indexes),
        )
        .route(
            "/management/integrity/{tenant}/rebuild",
            post(integrity::rebuild_indexes),
        )
        .route(
            "/management/integrity/{tenant}/rebuild/start",
            post(integrity::start_rebuild_indexes),
        )
        .route(
            "/management/integrity/{tenant}/cleanup",
            post(integrity::cleanup_orphans),
        )
        .route(
            "/management/integrity/{tenant}/cleanup/start",
            post(integrity::start_cleanup_orphans),
        )
        .route(
            "/management/integrity/{tenant}/cleanup-property-indexes",
            post(integrity::cleanup_property_index_orphans),
        )
        // Metrics endpoints
        .route("/management/metrics", get(maintenance::get_metrics))
        .route(
            "/management/metrics/{tenant}",
            get(maintenance::get_tenant_metrics),
        )
        // Maintenance endpoints
        .route("/management/compact", post(maintenance::trigger_compaction))
        .route(
            "/management/compact/start",
            post(maintenance::start_compaction),
        )
        .route(
            "/management/compact/{tenant}",
            post(maintenance::trigger_tenant_compaction),
        )
        // Backup endpoints
        .route("/management/backup/{tenant}", post(backup::backup_tenant))
        .route("/management/backup/all", post(backup::backup_all))
        .route("/management/backup/all/start", post(backup::start_backup))
        // Job management endpoints
        .route("/management/jobs", get(jobs::list_jobs))
        .route("/management/jobs/{id}", get(jobs::get_job_status))
        .route("/management/jobs/{id}/info", get(jobs::get_job_info))
        .route(
            "/management/jobs/{id}",
            axum::routing::delete(jobs::delete_job),
        )
        .route("/management/jobs/{id}/cancel", post(jobs::cancel_job))
        .route(
            "/management/jobs/batch-delete",
            post(jobs::batch_delete_jobs),
        )
        .route(
            "/management/jobs/schedule/integrity",
            post(jobs::schedule_integrity_scan),
        )
        // Job queue management endpoints
        .route("/management/jobs/stats", get(jobs::get_job_queue_stats))
        .route("/management/jobs/purge-all", post(jobs::purge_all_jobs))
        .route(
            "/management/jobs/purge-orphaned",
            post(jobs::purge_orphaned_jobs),
        )
        .route(
            "/management/jobs/force-fail-stuck",
            post(jobs::force_fail_stuck_jobs),
        )
        // SSE streaming endpoints for real-time updates
        .route(
            "/management/events/jobs",
            get(crate::sse::job_events_stream::<S>),
        )
        .route(
            "/management/events/health",
            get(crate::sse::health_events_stream::<S>),
        )
        .route(
            "/management/events/metrics",
            get(crate::sse::metrics_events_stream::<S>),
        )
        .with_state(state)
}
