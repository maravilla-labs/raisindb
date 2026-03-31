//! Graph cache state and background task management.

use raisin_rocksdb::{
    graph::{GraphCacheLayer, GraphComputeConfig, GraphComputeTask},
    RocksDBStorage,
};
use std::sync::Arc;
use tokio::sync::broadcast;

use super::super::ManagementState;
use super::types::GraphCacheEvent;

/// State for graph cache management endpoints
#[derive(Clone)]
pub struct GraphCacheState {
    pub storage: Arc<RocksDBStorage>,
    pub cache_layer: Option<Arc<GraphCacheLayer>>,
    /// Broadcast channel for SSE events
    pub event_sender: broadcast::Sender<GraphCacheEvent>,
    /// Configuration for background computation
    pub compute_config: GraphComputeConfig,
    /// Next tick timestamp (Unix millis)
    pub next_tick_at: Arc<std::sync::atomic::AtomicU64>,
}

impl GraphCacheState {
    /// Create from ManagementState with optional cache layer
    pub fn from_management_state(
        state: ManagementState<RocksDBStorage>,
        cache_layer: Option<Arc<GraphCacheLayer>>,
    ) -> Self {
        let (event_sender, _) = broadcast::channel(1024);
        Self {
            storage: state.storage,
            cache_layer,
            event_sender,
            compute_config: GraphComputeConfig::default(),
            next_tick_at: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    /// Create with explicit parameters
    pub fn new(
        storage: Arc<RocksDBStorage>,
        cache_layer: Option<Arc<GraphCacheLayer>>,
        compute_config: GraphComputeConfig,
    ) -> Self {
        let (event_sender, _) = broadcast::channel(1024);
        Self {
            storage,
            cache_layer,
            event_sender,
            compute_config,
            next_tick_at: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    /// Update next tick timestamp
    pub fn set_next_tick(&self, timestamp_ms: u64) {
        self.next_tick_at
            .store(timestamp_ms, std::sync::atomic::Ordering::SeqCst);
    }

    /// Get next tick timestamp
    pub fn get_next_tick(&self) -> u64 {
        self.next_tick_at.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Broadcast an event to all SSE clients
    pub fn broadcast(&self, event: GraphCacheEvent) {
        // Ignore errors (no subscribers)
        let _ = self.event_sender.send(event);
    }

    /// Get the tick interval in seconds
    pub fn tick_interval_secs(&self) -> u64 {
        self.compute_config.check_interval.as_secs()
    }
}

/// Start the graph cache background task
///
/// This spawns a tokio task that periodically checks for stale graph caches
/// and recomputes them. It also broadcasts events to SSE clients.
///
/// Returns the shared state that should be added as an Extension to the router.
pub fn start_graph_cache_background_task(
    storage: Arc<RocksDBStorage>,
    config: GraphComputeConfig,
) -> Arc<GraphCacheState> {
    let cache_layer = Arc::new(GraphCacheLayer::new());
    let state = Arc::new(GraphCacheState::new(
        storage.clone(),
        Some(cache_layer.clone()),
        config.clone(),
    ));

    if !config.enabled {
        tracing::info!("Graph cache background task is disabled");
        return state;
    }

    let state_clone = state.clone();
    let tick_interval = config.check_interval;

    tokio::spawn(async move {
        tracing::info!(
            "Starting graph cache background task (interval: {:?})",
            tick_interval
        );

        loop {
            // Calculate and set next tick time
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let next_tick_at = now + (tick_interval.as_millis() as u64);
            state_clone.set_next_tick(next_tick_at);

            // Wait for the tick interval
            tokio::time::sleep(tick_interval).await;

            tracing::debug!("Graph cache background tick starting");

            // Run the tick using the static method from GraphComputeTask
            match GraphComputeTask::run_tick_static(
                &state_clone.storage,
                state_clone.cache_layer.as_ref().unwrap(),
                &state_clone.compute_config,
            )
            .await
            {
                Ok(tick_stats) => {
                    if tick_stats.configs_processed > 0 {
                        tracing::info!(
                            configs_processed = tick_stats.configs_processed,
                            nodes_computed = tick_stats.nodes_computed,
                            errors = tick_stats.errors,
                            "Graph cache background tick completed"
                        );
                    } else {
                        tracing::debug!("Graph cache background tick completed (no work)");
                    }
                }
                Err(e) => {
                    tracing::error!("Graph cache background tick failed: {}", e);
                }
            }
        }
    });

    tracing::info!("Graph cache background task spawned");
    state
}
