//! P2P Cluster Catch-Up Coordinator
//!
//! This module implements the masterless cluster catch-up protocol that allows
//! a fresh node to join a P2P cluster and synchronize the complete state from peers.
//!
//! ## Catch-Up Protocol Phases
//!
//! 1. **Cluster Discovery**: Query all seed peers for cluster status
//! 2. **Consensus Determination**: Calculate consensus log index (CLI) from peer responses
//! 3. **Source Selection**: Select peer with most up-to-date state
//! 4. **Initiate Catch-Up**: Establish catch-up session with source peer
//! 5. **Transfer Checkpoint**: Stream RocksDB SST files (bulk data transfer)
//! 6. **Transfer Indexes**: Stream Tantivy fulltext and HNSW vector indexes
//! 7. **Log Verification**: Query all peers for operations beyond CLI, resolve conflicts
//! 8. **Node Ready**: Announce readiness and transition to steady-state replication
//!
// NOTE: This file intentionally exceeds 300 lines because the CatchUpCoordinator struct,
// its constructor, and the 8-phase orchestration method (execute_full_catch_up) are tightly
// coupled and cannot be meaningfully split without fragmenting the protocol flow. Each
// protocol phase implementation is already extracted into its own submodule.

mod checkpoint;
mod discovery;
mod hnsw_transfer;
mod index_transfer;
mod log_verification;
mod session;
mod tantivy_transfer;
pub mod types;

pub use types::{
    CatchUpResult, CatchUpSession, CheckpointTransferResult, ConsensusState, IndexTransferResult,
    PeerStatus, VerificationResult,
};

use crate::{
    ConflictResolver, OperationLogStorage, ParallelTransferOrchestrator, ReplicationMessage,
    VectorClock,
};
use raisin_error::{Error, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Catch-up coordinator for P2P cluster synchronization
pub struct CatchUpCoordinator {
    /// Local node ID
    local_node_id: String,

    /// Seed peer addresses for initial discovery
    seed_peers: Vec<String>,

    /// Local data directory (where RocksDB will be)
    data_dir: PathBuf,

    /// Staging directory for incoming transfers
    staging_dir: PathBuf,

    /// Conflict resolver for divergent logs
    conflict_resolver: Arc<ConflictResolver>,

    /// File streaming orchestrator
    transfer_orchestrator: Arc<ParallelTransferOrchestrator>,

    /// Storage backend for applying operations
    storage: Option<Arc<dyn OperationLogStorage>>,

    /// Checkpoint ingestor for applying received checkpoints
    checkpoint_ingestor: Option<Arc<dyn crate::CheckpointIngestor>>,

    /// Tantivy index receiver for ingesting fulltext indexes
    tantivy_receiver: Option<Arc<dyn crate::TantivyIndexReceiver>>,

    /// HNSW index receiver for ingesting vector indexes
    hnsw_receiver: Option<Arc<dyn crate::HnswIndexReceiver>>,

    /// Checkpoint threshold for hybrid strategy
    /// If operation delta > threshold, use checkpoint + tail replay
    /// Otherwise, use full log replay only
    checkpoint_threshold: usize,

    /// Timeout for network operations
    network_timeout: Duration,

    /// Active peer connections
    peer_connections: Arc<RwLock<HashMap<String, TcpStream>>>,
}

impl CatchUpCoordinator {
    /// Create a new catch-up coordinator
    ///
    /// # Arguments
    /// * `local_node_id` - ID of this node
    /// * `seed_peers` - List of seed peer addresses (e.g., "127.0.0.1:9001")
    /// * `data_dir` - Local data directory path
    /// * `staging_dir` - Staging directory for transfers
    /// * `storage` - Optional storage backend for applying operations
    /// * `checkpoint_ingestor` - Optional checkpoint ingestor for applying received checkpoints
    /// * `tantivy_receiver` - Optional Tantivy index receiver for ingesting fulltext indexes
    /// * `hnsw_receiver` - Optional HNSW index receiver for ingesting vector indexes
    /// * `checkpoint_threshold` - Threshold for using checkpoint vs full replay (default: 10,000)
    pub fn new(
        local_node_id: String,
        seed_peers: Vec<String>,
        data_dir: PathBuf,
        staging_dir: PathBuf,
        storage: Option<Arc<dyn OperationLogStorage>>,
        checkpoint_ingestor: Option<Arc<dyn crate::CheckpointIngestor>>,
        tantivy_receiver: Option<Arc<dyn crate::TantivyIndexReceiver>>,
        hnsw_receiver: Option<Arc<dyn crate::HnswIndexReceiver>>,
        checkpoint_threshold: Option<usize>,
    ) -> Self {
        let conflict_resolver = Arc::new(ConflictResolver::new(local_node_id.clone()));
        let transfer_orchestrator = Arc::new(ParallelTransferOrchestrator::new(4));

        Self {
            local_node_id,
            seed_peers,
            data_dir,
            staging_dir,
            conflict_resolver,
            transfer_orchestrator,
            storage,
            checkpoint_ingestor,
            tantivy_receiver,
            hnsw_receiver,
            checkpoint_threshold: checkpoint_threshold.unwrap_or(10_000),
            network_timeout: Duration::from_secs(30),
            peer_connections: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Determine if checkpoint strategy should be used based on operation delta
    ///
    /// # Arguments
    /// * `source_log_index` - Log index from source peer
    /// * `local_log_index` - Local node's log index (typically 0 for fresh nodes)
    ///
    /// # Returns
    /// True if checkpoint + indexes + tail replay should be used,
    /// False if full log replay is more efficient
    fn should_use_checkpoint(&self, source_log_index: u64, local_log_index: u64) -> bool {
        let operation_delta = source_log_index.saturating_sub(local_log_index);
        let use_checkpoint = operation_delta > self.checkpoint_threshold as u64;

        info!(
            source_log_index = source_log_index,
            local_log_index = local_log_index,
            operation_delta = operation_delta,
            threshold = self.checkpoint_threshold,
            strategy = if use_checkpoint {
                "checkpoint + tail"
            } else {
                "full replay"
            },
            "Catch-up strategy determined"
        );

        use_checkpoint
    }

    /// Execute the complete catch-up protocol
    ///
    /// This is the main entry point that orchestrates all catch-up phases.
    pub async fn execute_full_catch_up(&self) -> Result<CatchUpResult> {
        info!(
            local_node_id = %self.local_node_id,
            num_seed_peers = self.seed_peers.len(),
            "Starting P2P cluster catch-up protocol"
        );

        // Phase 1: Cluster Discovery
        let peer_statuses = self.discover_cluster().await?;

        if peer_statuses.is_empty() {
            return Err(Error::Backend(
                "No peers responded to cluster discovery".to_string(),
            ));
        }

        info!(
            num_peers = peer_statuses.len(),
            "Cluster discovery complete"
        );

        // Phase 2: Consensus Determination
        let consensus = self.calculate_consensus(&peer_statuses)?;

        info!(
            consensus_log_index = consensus.log_index,
            "Consensus determined"
        );

        // Phase 3: Source Selection
        let source_peer = self.select_source_peer(&peer_statuses, &consensus)?;

        info!(
            source_peer_id = %source_peer.node_id,
            source_address = %source_peer.address,
            "Source peer selected for catch-up"
        );

        // Phase 4: Initiate Catch-Up
        let catch_up_session = self.initiate_catch_up(source_peer).await?;

        info!(
            session_id = %catch_up_session.session_id,
            "Catch-up session initiated"
        );

        // Determine whether checkpoint strategy is available/preferred
        let checkpoint_supported = self.checkpoint_ingestor.is_some();
        let mut use_checkpoint = if checkpoint_supported {
            info!("Checkpoint ingestor configured locally - defaulting to checkpoint catch-up");
            true
        } else {
            self.should_use_checkpoint(source_peer.log_index, 0)
        };

        if !checkpoint_supported && use_checkpoint {
            info!(
                "Checkpoint preferred by heuristic but ingestor unavailable - falling back to log replay"
            );
            use_checkpoint = false;
        }

        // Initialize result structures
        let mut checkpoint_result = CheckpointTransferResult {
            num_files: 0,
            total_bytes: 0,
            duration: Duration::from_secs(0),
        };

        let mut index_result = IndexTransferResult {
            tantivy_files: 0,
            hnsw_indexes: 0,
        };

        if use_checkpoint {
            let (cp_res, idx_res) = self
                .run_checkpoint_strategy(source_peer, &catch_up_session)
                .await?;
            checkpoint_result = cp_res;
            index_result = idx_res;
        } else {
            info!("Using full log replay strategy (skipping checkpoint and indexes)");
        }

        // Phase 7: Log Verification and Replay
        let verification_result = match self
            .verify_and_apply_log_tail(&peer_statuses, &consensus)
            .await
        {
            Ok(result) => result,
            Err(e) => {
                if use_checkpoint {
                    return Err(e);
                }

                if !checkpoint_supported {
                    warn!(
                        error = %e,
                        "Full log replay failed and checkpoint ingestion unavailable"
                    );
                    return Err(e);
                }

                warn!(
                    error = %e,
                    "Full log replay failed - falling back to checkpoint strategy"
                );

                let (cp_res, idx_res) = self
                    .run_checkpoint_strategy(source_peer, &catch_up_session)
                    .await?;
                checkpoint_result = cp_res;
                index_result = idx_res;

                self.verify_and_apply_log_tail(&peer_statuses, &consensus)
                    .await?
            }
        };

        info!(
            operations_applied = verification_result.operations_applied,
            conflicts_resolved = verification_result.conflicts_resolved,
            "Log verification and replay complete"
        );

        // Phase 8: Announce Node Ready
        self.announce_node_ready().await?;

        info!("Catch-up protocol complete, node is ready for steady-state replication");

        Ok(CatchUpResult {
            source_peer_id: source_peer.node_id.clone(),
            checkpoint_result,
            index_result,
            verification_result,
        })
    }

    /// Execute checkpoint + index transfer strategy
    async fn run_checkpoint_strategy(
        &self,
        source_peer: &PeerStatus,
        catch_up_session: &CatchUpSession,
    ) -> Result<(CheckpointTransferResult, IndexTransferResult)> {
        info!("Using checkpoint + indexes + tail replay strategy");

        // Phase 5: Transfer RocksDB Checkpoint
        let checkpoint_result = self
            .transfer_checkpoint(source_peer, catch_up_session)
            .await?;

        info!(
            num_sst_files = checkpoint_result.num_files,
            total_bytes = checkpoint_result.total_bytes,
            duration_secs = checkpoint_result.duration.as_secs(),
            "RocksDB checkpoint transfer complete"
        );

        // Phase 5.5: Ingest checkpoint into local database
        if let Some(ref ingestor) = self.checkpoint_ingestor {
            info!(
                session_id = %catch_up_session.session_id,
                "Ingesting checkpoint into local database"
            );

            let checkpoint_dir = self.staging_dir.join(&catch_up_session.session_id);
            let num_files_ingested = ingestor
                .ingest_checkpoint(&checkpoint_dir, &catch_up_session.session_id)
                .await
                .map_err(|e| Error::Backend(format!("Checkpoint ingestion failed: {}", e)))?;

            info!(
                num_files_ingested = num_files_ingested,
                "Checkpoint ingestion complete"
            );
        } else {
            warn!("No checkpoint ingestor configured, skipping checkpoint ingestion");
        }

        // Phase 6: Transfer Indexes
        let index_result = self.transfer_indexes(source_peer, catch_up_session).await?;

        info!(
            num_tantivy_files = index_result.tantivy_files,
            num_hnsw_indexes = index_result.hnsw_indexes,
            "Index transfer complete"
        );

        Ok((checkpoint_result, index_result))
    }

    /// Phase 8: Announce node ready to all peers
    async fn announce_node_ready(&self) -> Result<()> {
        info!("Phase 8: Announcing node ready to all peers");

        let announcement = ReplicationMessage::NodeReady {
            node_id: self.local_node_id.clone(),
            vector_clock: VectorClock::new(), // TODO: Use actual vector clock
        };

        // Send to all known peers
        let connections = self.peer_connections.read().await;
        for (peer_id, _) in connections.iter() {
            info!(peer_id = %peer_id, "Announcing ready to peer");
            // TODO: Send announcement
        }

        Ok(())
    }

    /// Helper: Best-effort snapshot of this node's current vector clock
    async fn local_vector_clock(&self) -> VectorClock {
        if let Some(storage) = &self.storage {
            match storage.get_cluster_stats().await {
                Ok(stats) => stats.max_vector_clock,
                Err(e) => {
                    warn!(
                        error = %e,
                        "Failed to read local vector clock snapshot, using empty clock"
                    );
                    VectorClock::new()
                }
            }
        } else {
            VectorClock::new()
        }
    }

    /// Helper: Send a protocol message
    ///
    /// Uses shared TCP helpers for MessagePack serialization
    async fn send_message(stream: &mut TcpStream, message: &ReplicationMessage) -> Result<()> {
        crate::tcp_helpers::send_message(stream, message).await
    }

    /// Helper: Receive a protocol message
    ///
    /// Uses shared TCP helpers for MessagePack deserialization
    async fn receive_message(stream: &mut TcpStream) -> Result<ReplicationMessage> {
        crate::tcp_helpers::receive_message(stream).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_consensus_calculation() {
        let temp_data = TempDir::new().unwrap();
        let temp_staging = TempDir::new().unwrap();

        let coordinator = CatchUpCoordinator::new(
            "node1".to_string(),
            vec![],
            temp_data.path().to_path_buf(),
            temp_staging.path().to_path_buf(),
            None, // No storage backend for test
            None, // No checkpoint ingestor for test
            None, // No Tantivy receiver for test
            None, // No HNSW receiver for test
            None, // Default checkpoint threshold
        );

        let mut vc1 = VectorClock::new();
        vc1.set("node1", 10);

        let mut vc2 = VectorClock::new();
        vc2.set("node1", 8);
        vc2.set("node2", 5);

        let peers = vec![
            PeerStatus {
                node_id: "peer1".to_string(),
                address: "127.0.0.1:9001".to_string(),
                log_index: 100,
                vector_clock: vc1,
                num_tenants: 5,
                num_repos: 20,
                last_update_timestamp_ms: 1000000,
                known_peers: vec![],
                tenant_repos: vec![],
            },
            PeerStatus {
                node_id: "peer2".to_string(),
                address: "127.0.0.1:9002".to_string(),
                log_index: 95,
                vector_clock: vc2,
                num_tenants: 5,
                num_repos: 20,
                last_update_timestamp_ms: 999000,
                known_peers: vec![],
                tenant_repos: vec![],
            },
        ];

        let consensus = coordinator.calculate_consensus(&peers).unwrap();

        // Median of [95, 100] = 97.5 => 97
        assert_eq!(consensus.log_index, 97);

        // Consensus clock should have max of each node
        assert_eq!(consensus.vector_clock.get("node1"), 10);
        assert_eq!(consensus.vector_clock.get("node2"), 5);
    }

    #[tokio::test]
    async fn test_source_peer_selection() {
        let temp_data = TempDir::new().unwrap();
        let temp_staging = TempDir::new().unwrap();

        let coordinator = CatchUpCoordinator::new(
            "node1".to_string(),
            vec![],
            temp_data.path().to_path_buf(),
            temp_staging.path().to_path_buf(),
            None, // No storage backend for test
            None, // No checkpoint ingestor for test
            None, // No Tantivy receiver for test
            None, // No HNSW receiver for test
            None, // Default checkpoint threshold
        );

        let peers = vec![
            PeerStatus {
                node_id: "peer1".to_string(),
                address: "127.0.0.1:9001".to_string(),
                log_index: 100,
                vector_clock: VectorClock::new(),
                num_tenants: 5,
                num_repos: 20,
                last_update_timestamp_ms: 1000000,
                known_peers: vec![],
                tenant_repos: vec![],
            },
            PeerStatus {
                node_id: "peer2".to_string(),
                address: "127.0.0.1:9002".to_string(),
                log_index: 105, // Higher log index
                vector_clock: VectorClock::new(),
                num_tenants: 5,
                num_repos: 20,
                last_update_timestamp_ms: 1001000,
                known_peers: vec![],
                tenant_repos: vec![],
            },
        ];

        let consensus = ConsensusState {
            log_index: 100,
            vector_clock: VectorClock::new(),
        };

        let source = coordinator.select_source_peer(&peers, &consensus).unwrap();

        // Should select peer2 (higher log index)
        assert_eq!(source.node_id, "peer2");
        assert_eq!(source.log_index, 105);
    }
}
