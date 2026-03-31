//! Replication coordination and synchronization
//!
//! This module orchestrates the replication process between cluster nodes,
//! managing both periodic pull-based sync and real-time push-based replication.

mod lifecycle;
mod peer_queries;
mod push;
mod sync;
pub mod traits;
pub mod types;

use std::sync::Arc;
use std::time::Instant;

use tokio::sync::RwLock;

use crate::config::{ClusterConfig, SyncConfig};
use crate::peer_manager::PeerManager;
use crate::replay::ReplayEngine;

pub use traits::{
    CheckpointIngestor, CheckpointProvider, HnswIndexReceiver, IndexLister, OperationLogStorage,
    TantivyIndexReceiver,
};
use types::CoordinatorMetrics;
pub use types::{
    ClusterStorageStats, CoordinatorError, PeerClusterSnapshot, StorageError, SyncStats,
};

/// Coordinates replication synchronization between cluster peers
pub struct ReplicationCoordinator {
    /// This node's cluster ID
    pub(crate) cluster_node_id: String,

    /// Peer connection manager
    pub(crate) peer_manager: Arc<PeerManager>,

    /// Storage backend
    pub(crate) storage: Arc<dyn OperationLogStorage>,

    /// CRDT replay engine for applying operations
    pub(crate) replay_engine: Arc<RwLock<ReplayEngine>>,

    /// Causal delivery buffer - ensures operations are delivered in causal order
    pub(crate) causal_buffer: Arc<RwLock<crate::causal_delivery::CausalDeliveryBuffer>>,

    /// Sync configuration
    pub(crate) sync_config: SyncConfig,

    /// Optional checkpoint provider for serving RocksDB snapshots
    pub(crate) checkpoint_provider: Option<Arc<dyn CheckpointProvider>>,

    /// List of tenant/repo pairs to sync
    pub(crate) sync_tenants: Vec<(String, String)>,

    /// Metrics tracking
    pub(crate) metrics: CoordinatorMetrics,

    /// When coordinator was created
    pub(crate) started_at: Instant,
}

impl ReplicationCoordinator {
    /// Create a new replication coordinator with in-memory idempotency tracking
    ///
    /// For persistent idempotency tracking, use `new_with_persistent_idempotency()`.
    pub fn new(
        cluster_config: ClusterConfig,
        storage: Arc<dyn OperationLogStorage>,
    ) -> Result<Self, CoordinatorError> {
        let peer_manager = Arc::new(PeerManager::new(
            cluster_config.node_id.clone(),
            cluster_config.connection.clone(),
            cluster_config.sync.retry.clone(),
        ));

        // Initialize causal delivery buffer with empty vector clock
        // It will be updated with the actual vector clock during first sync
        let causal_buffer = Arc::new(RwLock::new(
            crate::causal_delivery::CausalDeliveryBuffer::new(
                crate::vector_clock::VectorClock::new(),
                Some(10_000), // Buffer up to 10,000 operations
            ),
        ));

        Ok(Self {
            cluster_node_id: cluster_config.node_id,
            peer_manager,
            storage,
            replay_engine: Arc::new(RwLock::new(ReplayEngine::new())),
            causal_buffer,
            sync_config: cluster_config.sync,
            checkpoint_provider: None,
            sync_tenants: cluster_config.sync_tenants,
            metrics: CoordinatorMetrics::new(),
            started_at: Instant::now(),
        })
    }

    /// Create a new replication coordinator with custom idempotency tracker
    ///
    /// This allows using persistent idempotency tracking or other custom implementations.
    ///
    /// # Arguments
    /// * `cluster_config` - Cluster configuration
    /// * `storage` - Operation log storage backend
    /// * `idempotency_tracker` - Custom idempotency tracker (e.g., PersistentIdempotencyTracker)
    pub fn new_with_tracker(
        cluster_config: ClusterConfig,
        storage: Arc<dyn OperationLogStorage>,
        idempotency_tracker: Box<dyn crate::IdempotencyTracker>,
    ) -> Result<Self, CoordinatorError> {
        let peer_manager = Arc::new(PeerManager::new(
            cluster_config.node_id.clone(),
            cluster_config.connection.clone(),
            cluster_config.sync.retry.clone(),
        ));

        // Initialize causal delivery buffer with empty vector clock
        let causal_buffer = Arc::new(RwLock::new(
            crate::causal_delivery::CausalDeliveryBuffer::new(
                crate::vector_clock::VectorClock::new(),
                Some(10_000),
            ),
        ));

        Ok(Self {
            cluster_node_id: cluster_config.node_id,
            peer_manager,
            storage,
            replay_engine: Arc::new(RwLock::new(ReplayEngine::with_tracker(idempotency_tracker))),
            causal_buffer,
            sync_config: cluster_config.sync,
            checkpoint_provider: None,
            sync_tenants: cluster_config.sync_tenants,
            metrics: CoordinatorMetrics::new(),
            started_at: Instant::now(),
        })
    }

    /// Set the checkpoint provider for serving RocksDB snapshots during catch-up
    ///
    /// This must be called before starting the coordinator if checkpoint serving is required.
    pub fn set_checkpoint_provider(&mut self, checkpoint_provider: Arc<dyn CheckpointProvider>) {
        self.checkpoint_provider = Some(checkpoint_provider);
    }
}

// Need Clone for spawning tasks
impl Clone for ReplicationCoordinator {
    fn clone(&self) -> Self {
        Self {
            cluster_node_id: self.cluster_node_id.clone(),
            peer_manager: self.peer_manager.clone(),
            storage: self.storage.clone(),
            replay_engine: self.replay_engine.clone(),
            causal_buffer: self.causal_buffer.clone(),
            sync_config: self.sync_config.clone(),
            checkpoint_provider: self.checkpoint_provider.clone(),
            sync_tenants: self.sync_tenants.clone(),
            metrics: self.metrics.clone(),
            started_at: self.started_at,
        }
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
