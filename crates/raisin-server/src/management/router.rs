//! Router construction for the management API.
//!
//! Provides two versions of the router:
//! - RocksDB-specific: with concrete storage types, monitoring, auth middlewares
//! - Generic: for other storage backends implementing `ManagementOps + BackgroundJobs`
//!
//! ## Tenant scoping
//!
//! Every `/management/*` route is tenant-scoped — either by a `{tenant}` path
//! segment, or by reading `TenantInfo` (populated from the `x-tenant-id`
//! header) in the handler. Truly global / cross-tenant operations live under
//! `/management/admin/*` and are gated by `require_superadmin_token_middleware`.
//!
//! Customer-facing callers must never hit `/management/admin/*`. Calling a
//! global endpoint on the customer-facing path returns 404 (the old route is
//! deleted, not aliased).

use axum::{
    routing::{get, post},
    Router,
};
use raisin_storage::{BackgroundJobs, ManagementOps};
use std::sync::Arc;

#[cfg(feature = "storage-rocksdb")]
use super::admin;
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
/// - `require_superadmin_token_middleware` (admin subtree only)
#[cfg(feature = "storage-rocksdb")]
pub fn management_router(
    storage: Arc<raisin_rocksdb::RocksDBStorage>,
    monitoring: Arc<raisin_rocksdb::monitoring::MonitoringService>,
    graph_cache_state: Option<Arc<graph_cache::GraphCacheState>>,
    hnsw_engine: Option<Arc<raisin_hnsw::HnswIndexingEngine>>,
    app_state: raisin_transport_http::state::AppState,
) -> Router {
    use axum::{
        middleware::{from_fn, from_fn_with_state},
        Extension,
    };
    use raisin_transport_http::middleware::{
        ensure_tenant_middleware, require_admin_auth_middleware,
        require_superadmin_token_middleware,
    };

    // Get data_dir before storage is moved into state
    let data_dir = storage.config().path.to_string_lossy().to_string();
    // The shutdown token rides along from the HTTP state so the SSE handlers
    // mounted below can end their streams instead of holding the drain.
    let shutdown = app_state.shutdown_token();
    let state = ManagementState {
        storage: storage.clone(),
        shutdown: shutdown.clone(),
    };
    let state_for_admin = ManagementState {
        storage: storage.clone(),
        shutdown,
    };
    drop(storage);

    // ------------------------------------------------------------------
    // Per-tenant routes (always mounted)
    //
    // Every route below is scoped to a single tenant — either via `{tenant}`
    // in the path, or via `Extension<TenantInfo>` derived from the request's
    // `x-tenant-id` header. Cross-tenant ops live under `/management/admin/*`.
    // ------------------------------------------------------------------
    let router = Router::new()
        // Health endpoints (per-tenant only on the customer-facing plane).
        .route(
            "/management/health/{tenant}",
            get(health::get_tenant_health),
        )
        // Integrity endpoints — all tenant-scoped in the URL.
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
        // Per-tenant metrics — server-wide metrics live under /management/admin.
        .route(
            "/management/metrics/{tenant}",
            get(maintenance::get_tenant_metrics),
        )
        // Per-tenant maintenance — global compaction lives under /management/admin.
        .route(
            "/management/compact/{tenant}",
            post(maintenance::trigger_tenant_compaction),
        )
        .route(
            "/management/compact/{tenant}/start",
            post(maintenance::start_tenant_compaction),
        )
        // Per-tenant backup — full-instance backup lives under /management/admin.
        // Shadow the literal `/backup/all` path so axum's `{tenant}` matcher
        // does not silently route it to `backup_tenant("all")` and leak a
        // 401-vs-404 signal about a non-existent tenant.
        .route(
            "/management/backup/all",
            post(|| async { axum::http::StatusCode::NOT_FOUND }),
        )
        .route("/management/backup/{tenant}", post(backup::backup_tenant))
        // Job management endpoints — tenant-scoped via `Extension<TenantInfo>`.
        // Point lookups 404 on cross-tenant access (no info leak); aggregate
        // ops filter by tenant. The true cross-tenant variants live under
        // `/management/admin/jobs/*`.
        .route("/management/jobs", get(jobs::list_jobs))
        .route(
            "/management/jobs/history/backfill",
            post(jobs::backfill_job_history_index),
        )
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
        // Job queue management endpoints — also tenant-scoped via header.
        .route("/management/jobs/stats", get(jobs::get_job_queue_stats))
        // This tenant's degraded bit + its own queue depth. Static segment, so
        // it is matched ahead of `/management/jobs/{id}` — same as `stats`
        // above. The breaker keys, failure counts, probe timers and host-wide
        // pool figures this route used to return are cross-tenant and now live
        // at `/management/admin/jobs/health`.
        .route("/management/jobs/health", get(jobs::get_tenant_job_health))
        .route("/management/jobs/purge-all", post(jobs::purge_all_jobs))
        .route(
            "/management/jobs/purge-orphaned",
            post(jobs::purge_orphaned_jobs),
        )
        .route(
            "/management/jobs/force-fail-stuck",
            post(jobs::force_fail_stuck_jobs),
        )
        // SSE streaming endpoints — filtered to the caller's tenant.
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
        // Graph cache management endpoints — repo-scoped (no global form).
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
        .layer(Extension(monitoring.clone()));

    // Add graph cache state as Extension if available
    let router = if let Some(gcs) = graph_cache_state {
        router.layer(Extension(gcs))
    } else {
        router
    };

    // Operator package provisioning (upload + unified install command) — the
    // same handlers as the customer /api package surface, mounted here so
    // they inherit the admin-auth gate below (per-tenant admin JWT, or the
    // operator superadmin bearer when configured). Lets the hosting control
    // plane install packages into a tenant without touching customer-facing
    // auth; self-hosted deployments still require a valid admin JWT here.
    let router = router.merge(raisin_transport_http::operator_package_routes(
        app_state.clone(),
    ));

    // Apply security middlewares to the per-tenant router
    // ensure_tenant runs FIRST (outer), then require_admin (inner)
    // In Axum layers, later layers run first, so add require_admin first
    let router = router
        .layer(from_fn_with_state(
            app_state.clone(),
            require_admin_auth_middleware,
        ))
        .layer(from_fn_with_state(
            app_state.clone(),
            ensure_tenant_middleware,
        ));

    // Conditionally mount the superadmin subtree. If the env var is unset or
    // empty, /management/admin/* routes are not registered at all (404 instead
    // of exposing the env-var probe surface).
    let superadmin_enabled = std::env::var("RAISIN_SUPERADMIN_TOKEN")
        .map(|v| !v.is_empty())
        .unwrap_or(false);

    if !superadmin_enabled {
        return router;
    }

    // ------------------------------------------------------------------
    // /management/admin/* — superadmin-token gated, cross-tenant
    // ------------------------------------------------------------------

    // Tenants + reset-password — use AppState (auth_service + storage).
    let admin_app_router = Router::new()
        .route(
            "/management/admin/tenants",
            post(admin::tenants::provision_tenant),
        )
        .route(
            "/management/admin/tenants/{tenant_id}",
            axum::routing::delete(admin::tenants::delete_tenant),
        )
        .route(
            "/management/admin/reset-password",
            post(admin::passwords::reset_admin_password),
        )
        // Identity-user provisioning for a managed tenant. Scoped by path
        // param rather than `x-tenant-id`, because `ensure_tenant_middleware`
        // silently falls back to the "default" tenant when the header is
        // absent — too sharp an edge for a cross-tenant write.
        .route(
            "/management/admin/tenants/{tenant_id}/identity-users",
            post(admin::identity_users::create_identity_user)
                .get(admin::identity_users::list_identity_users),
        )
        .with_state(app_state.clone());

    // Server-wide health, metrics, maintenance, backup, jobs — use
    // ManagementState<RocksDBStorage>.
    let admin_global_router = Router::new()
        // Cross-tenant job ops.
        .route("/management/admin/jobs", get(admin::jobs::list_all_jobs))
        .route(
            "/management/admin/jobs/purge-all",
            post(admin::jobs::purge_all_global),
        )
        .route(
            "/management/admin/jobs/force-fail-stuck",
            post(admin::jobs::force_fail_stuck_global),
        )
        // The whole job-system picture: every upstream breaker, every category
        // pool, and per-tenant queue depth. Superadmin-only because all three
        // are cross-tenant — a breaker is shared by every tenant on the host,
        // and the tenant rows name other tenants outright. The per-tenant
        // `/management/jobs/health` answers the reduced shape instead.
        .route(
            "/management/admin/jobs/health",
            get(admin::jobs::get_job_system_health_global),
        )
        // Server-wide health (moved from /management/health).
        .route(
            "/management/admin/health",
            get(health::get_health_with_monitoring),
        )
        .route("/management/admin/health/storage", get(health::get_health))
        // Server-wide metrics (moved from /management/metrics).
        .route("/management/admin/metrics", get(maintenance::get_metrics))
        // Memory diagnostics: allocator counters + per-CF RocksDB properties +
        // in-process collection sizes. Safe to poll; see `crate::diagnostics`.
        .route(
            "/management/admin/diagnostics/memory",
            get(maintenance::get_memory_diagnostics),
        )
        .route(
            "/management/admin/diagnostics/malloc-stats",
            get(maintenance::get_malloc_stats),
        )
        .route(
            "/management/admin/metrics/replication",
            get(health::replication_metrics_handler),
        )
        // /management/admin/metrics/vector requires the HNSW extension —
        // layered below conditionally so this route only works when the
        // engine is actually present.
        .route(
            "/management/admin/metrics/vector",
            get(health::vector_metrics_handler),
        )
        // Cross-tenant maintenance (moved from /management/compact[/start]).
        .route(
            "/management/admin/compact",
            post(maintenance::trigger_compaction),
        )
        .route(
            "/management/admin/compact/start",
            post(maintenance::start_compaction),
        )
        // Cross-tenant backup (moved from /management/backup/all[/start]).
        .route("/management/admin/backup/all", post(backup::backup_all))
        .route(
            "/management/admin/backup/all/start",
            post(backup::start_backup),
        )
        .with_state(state_for_admin.clone())
        // Monitoring extension is required by replication_metrics_handler and
        // get_health_with_monitoring on this sub-router.
        .layer(Extension(monitoring));

    // Layer the HNSW engine onto the admin sub-router conditionally. When
    // disabled (HNSW feature off or engine not initialised) the
    // /management/admin/metrics/vector route returns 500 (missing-extension)
    // on call — acceptable since the feature isn't running anyway and the
    // route still 404s the customer-facing path.
    let admin_global_router = if let Some(hnsw) = hnsw_engine {
        admin_global_router.layer(Extension(hnsw))
    } else {
        admin_global_router
    };

    // Dependencies — server-wide config, uses its own DepsState.
    let deps_state = super::dependencies::DepsState { data_dir };
    let admin_deps_router = Router::new()
        .route(
            "/management/admin/dependencies",
            get(super::dependencies::list_dependencies),
        )
        .route(
            "/management/admin/dependencies/{name}/enable",
            post(super::dependencies::enable_dependency),
        )
        .with_state(deps_state);

    let admin_router = admin_app_router
        .merge(admin_global_router)
        .merge(admin_deps_router)
        .layer(from_fn_with_state(app_state, ensure_tenant_middleware))
        .layer(from_fn(require_superadmin_token_middleware));

    router.merge(admin_router)
}

/// Create the management API router (generic version for other storage backends).
#[cfg(not(feature = "storage-rocksdb"))]
pub fn management_router<S>(storage: Arc<S>) -> Router
where
    S: ManagementOps + BackgroundJobs + Clone + Send + Sync + 'static,
{
    let state = ManagementState {
        storage,
        shutdown: None,
    };

    Router::new()
        // Per-tenant health (server-wide is operator-only).
        .route(
            "/management/health/{tenant}",
            get(health::get_tenant_health),
        )
        // Integrity endpoints — all tenant-scoped in the URL.
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
        // Per-tenant metrics (server-wide is operator-only).
        .route(
            "/management/metrics/{tenant}",
            get(maintenance::get_tenant_metrics),
        )
        // Per-tenant maintenance (global is operator-only).
        .route(
            "/management/compact/{tenant}",
            post(maintenance::trigger_tenant_compaction),
        )
        // Per-tenant backup (full-instance is operator-only).
        .route("/management/backup/{tenant}", post(backup::backup_tenant))
        // Job management endpoints — tenant-scoped via header.
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
        // Job queue management endpoints — also tenant-scoped via header.
        .route("/management/jobs/stats", get(jobs::get_job_queue_stats))
        // This tenant's degraded bit + its own queue depth. Static segment, so
        // it is matched ahead of `/management/jobs/{id}` — same as `stats`
        // above. The breaker keys, failure counts, probe timers and host-wide
        // pool figures this route used to return are cross-tenant and now live
        // at `/management/admin/jobs/health`.
        .route("/management/jobs/health", get(jobs::get_tenant_job_health))
        .route("/management/jobs/purge-all", post(jobs::purge_all_jobs))
        .route(
            "/management/jobs/purge-orphaned",
            post(jobs::purge_orphaned_jobs),
        )
        .route(
            "/management/jobs/force-fail-stuck",
            post(jobs::force_fail_stuck_jobs),
        )
        // SSE streaming endpoints — filtered to the caller's tenant.
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
