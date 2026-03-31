//! Type definitions, error types, and data structures for the coordinator.
//!
//! Contains error enums, statistics structs, metrics, and peer snapshot types
//! used throughout the coordinator module.

use crate::metrics::{AtomicCounter, DurationHistogram};
use crate::peer_manager::PeerManagerError;
use crate::VectorClock;

/// Cluster-wide storage statistics
#[derive(Debug, Clone)]
pub struct ClusterStorageStats {
    /// Aggregated vector clock (max across all tenant/repo pairs)
    pub max_vector_clock: VectorClock,
    /// Number of unique tenants
    pub num_tenants: usize,
    /// Number of repositories
    pub num_repos: usize,
    /// All (tenant_id, repo_id) pairs
    pub tenant_repos: Vec<(String, String)>,
}

/// Snapshot of a peer's cluster-wide replication state
#[derive(Debug, Clone)]
pub struct PeerClusterSnapshot {
    /// Peer ID as configured locally (from cluster config)
    pub configured_peer_id: String,
    /// Cluster node ID reported by the peer
    pub node_id: String,
    /// Peer log index (monotonic counter of applied operations)
    pub log_index: u64,
    /// Peer vector clock
    pub vector_clock: VectorClock,
    /// Number of tenants hosted by the peer
    pub num_tenants: usize,
    /// Number of repositories hosted by the peer
    pub num_repos: usize,
    /// Detailed tenant/repository listing reported by the peer
    pub tenant_repos: Vec<(String, String)>,
}

/// Sync statistics
#[derive(Debug, Clone)]
pub struct SyncStats {
    pub cluster_node_id: String,
    pub total_peers: usize,
    pub connected_peers: usize,
    pub disconnected_peers: usize,
}

/// Metrics for replication coordinator
#[derive(Debug, Clone)]
pub(crate) struct CoordinatorMetrics {
    pub(super) operations_pushed: AtomicCounter,
    pub(super) operations_received: AtomicCounter,
    pub(super) operations_applied: AtomicCounter,
    pub(super) operations_failed: AtomicCounter,
    pub(super) sync_cycles: AtomicCounter,
    pub(super) catch_up_triggered: AtomicCounter,
    pub(super) conflicts_detected: AtomicCounter,
    pub(super) operations_skipped: AtomicCounter,
    pub(super) sync_duration: DurationHistogram,
}

impl CoordinatorMetrics {
    pub(super) fn new() -> Self {
        Self {
            operations_pushed: AtomicCounter::new(),
            operations_received: AtomicCounter::new(),
            operations_applied: AtomicCounter::new(),
            operations_failed: AtomicCounter::new(),
            sync_cycles: AtomicCounter::new(),
            catch_up_triggered: AtomicCounter::new(),
            conflicts_detected: AtomicCounter::new(),
            operations_skipped: AtomicCounter::new(),
            sync_duration: DurationHistogram::new(1000),
        }
    }
}

/// Storage backend errors
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("Storage error: {0}")]
    Backend(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Serialization error: {0}")]
    Serialization(String),
}

/// Coordinator errors
#[derive(Debug, thiserror::Error)]
pub enum CoordinatorError {
    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Network error: {0}")]
    Network(String),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Catch-up error: {0}")]
    CatchUp(String),
}

impl From<PeerManagerError> for CoordinatorError {
    fn from(e: PeerManagerError) -> Self {
        CoordinatorError::Network(e.to_string())
    }
}
