//! HTTP handler functions for graph cache management API.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        Json,
    },
    Extension,
};
use futures::stream::Stream;
use raisin_rocksdb::{
    graph::{GraphCacheLayer, GraphComputeTask},
    RocksDBStorage,
};
use raisin_transport_http::middleware::TenantInfo;
use std::{convert::Infallible, sync::Arc, time::Duration};

use super::super::ManagementState;
use super::helpers::{config_to_response, load_all_configs, read_cache_meta, status_to_string};
use super::state::GraphCacheState;
use super::types::{ApiResponse, ConfigStatus, ConfigStatusResponse, GraphCacheEvent};

/// Default tick interval (60 seconds)
const DEFAULT_TICK_INTERVAL_SECS: u64 = 60;

/// GET /management/graph-cache/{repo}/status
/// Returns status of all graph algorithm configs for the repository
pub async fn get_graph_cache_status(
    State(state): State<ManagementState<RocksDBStorage>>,
    Extension(tenant_info): Extension<TenantInfo>,
    Path(repo_id): Path<String>,
) -> Result<Json<ApiResponse<ConfigStatusResponse>>, StatusCode> {
    let tenant_id = tenant_info.tenant_id.clone();

    // Load all configs from storage
    let configs = match load_all_configs(&state.storage, &tenant_id, &repo_id).await {
        Ok(configs) => configs,
        Err(e) => {
            tracing::error!("Failed to load graph configs: {}", e);
            return Ok(Json(ApiResponse::err(format!(
                "Failed to load configs: {}",
                e
            ))));
        }
    };

    // Get branches to check status for each config
    let branches =
        match raisin_rocksdb::management::list_branches(&state.storage, &tenant_id, &repo_id).await
        {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("Failed to list branches: {}", e);
                vec!["main".to_string()]
            }
        };

    let mut config_statuses = Vec::new();

    for config in configs {
        // Find the first branch this config targets
        let target_branch = branches
            .iter()
            .find(|b| config.targets_branch(b))
            .cloned()
            .unwrap_or_else(|| "main".to_string());

        // Read cache metadata
        let meta = read_cache_meta(
            &state.storage,
            &tenant_id,
            &repo_id,
            &target_branch,
            &config.id,
        );

        let (status, last_computed_at, next_scheduled_at, node_count, error) = match meta {
            Ok(Some(meta)) => (
                status_to_string(&meta.status),
                Some(meta.last_computed_at),
                if meta.next_scheduled_at > 0 {
                    Some(meta.next_scheduled_at)
                } else {
                    None
                },
                Some(meta.node_count),
                meta.error,
            ),
            Ok(None) => ("pending".to_string(), None, None, None, None),
            Err(e) => ("error".to_string(), None, None, None, Some(e.to_string())),
        };

        config_statuses.push(ConfigStatus {
            id: config.id.clone(),
            algorithm: config.algorithm.to_string(),
            enabled: config.enabled,
            status,
            last_computed_at,
            next_scheduled_at,
            node_count,
            error,
            config: config_to_response(&config),
        });
    }

    // Calculate next tick (simplified - actual value would come from background task)
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let next_tick_at = now + (DEFAULT_TICK_INTERVAL_SECS * 1000);

    let response = ConfigStatusResponse {
        configs: config_statuses,
        next_tick_at,
        tick_interval_seconds: DEFAULT_TICK_INTERVAL_SECS,
    };

    Ok(Json(ApiResponse::ok(response)))
}

/// POST /management/graph-cache/{repo}/{config_id}/recompute
/// Trigger immediate recomputation for a specific config
pub async fn trigger_recompute(
    State(state): State<ManagementState<RocksDBStorage>>,
    Extension(tenant_info): Extension<TenantInfo>,
    Path((repo_id, config_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<String>>, StatusCode> {
    let tenant_id = tenant_info.tenant_id.clone();

    tracing::info!(
        "Triggering immediate recomputation for config '{}' in repo '{}'",
        config_id,
        repo_id
    );

    // Load the specific config
    let configs = match load_all_configs(&state.storage, &tenant_id, &repo_id).await {
        Ok(c) => c,
        Err(e) => {
            return Ok(Json(ApiResponse::err(format!(
                "Failed to load configs: {}",
                e
            ))));
        }
    };

    let config = match configs.into_iter().find(|c| c.id == config_id) {
        Some(c) => c,
        None => {
            return Ok(Json(ApiResponse::err(format!(
                "Config '{}' not found",
                config_id
            ))));
        }
    };

    // Get branches for this config
    let branches =
        match raisin_rocksdb::management::list_branches(&state.storage, &tenant_id, &repo_id).await
        {
            Ok(b) => b,
            Err(e) => {
                return Ok(Json(ApiResponse::err(format!(
                    "Failed to list branches: {}",
                    e
                ))));
            }
        };

    // Find target branches
    let target_branches: Vec<String> = branches
        .into_iter()
        .filter(|b| config.targets_branch(b))
        .collect();

    if target_branches.is_empty() {
        return Ok(Json(ApiResponse::err(
            "No target branches found for config",
        )));
    }

    // Create a cache layer for the computation
    let cache_layer = Arc::new(GraphCacheLayer::new());

    // Spawn background task for recomputation
    let storage = state.storage.clone();
    let config_id_clone = config_id.clone();
    let repo_id_clone = repo_id.clone();
    let max_nodes = 100_000usize; // Default max nodes

    tokio::spawn(async move {
        let start = std::time::Instant::now();
        let mut total_nodes = 0u64;

        for branch_id in &target_branches {
            tracing::info!(
                "Processing branch '{}' for config '{}'",
                branch_id,
                config_id_clone
            );

            match GraphComputeTask::recompute_for_branch(
                &storage,
                &cache_layer,
                &tenant_id,
                &repo_id_clone,
                branch_id,
                &config,
                max_nodes,
            )
            .await
            {
                Ok(node_count) => {
                    total_nodes += node_count as u64;
                    tracing::info!("Recomputed {} nodes for branch '{}'", node_count, branch_id);
                }
                Err(e) => {
                    tracing::error!("Recomputation failed for branch '{}': {}", branch_id, e);
                    return;
                }
            }
        }

        let duration_ms = start.elapsed().as_millis() as u64;
        tracing::info!(
            "Recomputation completed for config '{}': {} nodes in {}ms",
            config_id_clone,
            total_nodes,
            duration_ms
        );
    });

    Ok(Json(ApiResponse::ok(format!(
        "Recomputation triggered for config '{}'",
        config_id
    ))))
}

/// POST /management/graph-cache/{repo}/{config_id}/mark-stale
/// Mark cache as stale to be picked up at next background tick
pub async fn mark_stale(
    State(state): State<ManagementState<RocksDBStorage>>,
    Extension(tenant_info): Extension<TenantInfo>,
    Path((repo_id, config_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<String>>, StatusCode> {
    let tenant_id = tenant_info.tenant_id.clone();

    tracing::info!(
        "Marking cache as stale for config '{}' in repo '{}'",
        config_id,
        repo_id
    );

    // Get branches
    let branches =
        match raisin_rocksdb::management::list_branches(&state.storage, &tenant_id, &repo_id).await
        {
            Ok(b) => b,
            Err(e) => {
                return Ok(Json(ApiResponse::err(format!(
                    "Failed to list branches: {}",
                    e
                ))));
            }
        };

    let mut marked_count = 0;
    for branch_id in branches {
        if let Err(e) = GraphComputeTask::mark_stale(
            &state.storage,
            &tenant_id,
            &repo_id,
            &branch_id,
            &config_id,
        ) {
            tracing::warn!("Failed to mark stale for branch '{}': {}", branch_id, e);
        } else {
            marked_count += 1;
        }
    }

    Ok(Json(ApiResponse::ok(format!(
        "Marked {} branches as stale for config '{}'",
        marked_count, config_id
    ))))
}

/// GET /management/graph-cache/{repo}/stream
/// SSE endpoint for real-time graph cache updates
///
/// Subscribes to the shared GraphCacheState broadcast channel for real-time events
/// from the background computation task. Falls back to local countdown if no
/// shared state is available.
pub async fn graph_cache_events_stream(
    State(_state): State<ManagementState<RocksDBStorage>>,
    Extension(tenant_info): Extension<TenantInfo>,
    graph_state: Option<axum::Extension<Arc<GraphCacheState>>>,
    Path(repo_id): Path<String>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    tracing::debug!(
        tenant_id = %tenant_info.tenant_id,
        repo_id = %repo_id,
        "Graph cache SSE stream opened"
    );
    let tick_interval = DEFAULT_TICK_INTERVAL_SECS;

    // Try to get the shared state for real tick updates
    let shared_state = graph_state.map(|ext| ext.0);

    let stream = async_stream::stream! {
        let mut countdown_interval = tokio::time::interval(Duration::from_secs(1));

        // Subscribe to broadcast channel if available
        let mut broadcast_rx = shared_state.as_ref().map(|s| s.event_sender.subscribe());

        loop {
            tokio::select! {
                // Handle broadcast events from background task
                event = async {
                    if let Some(ref mut rx) = broadcast_rx {
                        rx.recv().await.ok()
                    } else {
                        // No broadcast channel, wait forever (countdown_interval will handle it)
                        std::future::pending::<Option<GraphCacheEvent>>().await
                    }
                } => {
                    if let Some(event) = event {
                        let event_type = match &event {
                            GraphCacheEvent::TickCountdown { .. } => "tick_countdown",
                            GraphCacheEvent::ComputationStarted { .. } => "computation_started",
                            GraphCacheEvent::ComputationProgress { .. } => "computation_progress",
                            GraphCacheEvent::ComputationCompleted { .. } => "computation_completed",
                            GraphCacheEvent::ComputationFailed { .. } => "computation_failed",
                            GraphCacheEvent::StatusChanged { .. } => "status_changed",
                        };
                        let data = serde_json::to_string(&event).unwrap_or_default();
                        yield Ok(Event::default().event(event_type).data(data));
                    }
                }

                // Periodic countdown tick (1 second)
                _ = countdown_interval.tick() => {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;

                    // Get real next_tick_at from shared state, or calculate fallback
                    let next_tick_at = shared_state
                        .as_ref()
                        .map(|s| s.get_next_tick())
                        .filter(|&t| t > 0)
                        .unwrap_or_else(|| now + (tick_interval * 1000));

                    // Calculate remaining seconds
                    let remaining = if next_tick_at > now {
                        (next_tick_at - now) / 1000
                    } else {
                        0
                    };

                    let event = GraphCacheEvent::TickCountdown {
                        next_tick_at,
                        seconds_remaining: remaining,
                    };
                    let data = serde_json::to_string(&event).unwrap_or_default();
                    yield Ok(Event::default().event("tick_countdown").data(data));
                }
            }
        }
    };

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text("keep-alive"),
    )
}
