//! Data types for the catch-up protocol.
//!
//! Contains all public structs representing peer status, consensus state,
//! session information, and protocol result types.

use crate::VectorClock;
use std::time::Duration;

/// Peer status information from cluster discovery
#[derive(Debug, Clone)]
pub struct PeerStatus {
    /// Peer node ID
    pub node_id: String,

    /// Peer network address
    pub address: String,

    /// Peer's current log index
    pub log_index: u64,

    /// Peer's vector clock
    pub vector_clock: VectorClock,

    /// Number of tenants on peer
    pub num_tenants: usize,

    /// Number of repositories on peer
    pub num_repos: usize,

    /// Timestamp of last update (ms since epoch)
    pub last_update_timestamp_ms: u64,

    /// Other peers known to this peer
    pub known_peers: Vec<String>,

    /// Tenant/repository pairs hosted on the peer
    pub tenant_repos: Vec<(String, String)>,
}

/// Consensus state calculated from peer responses
#[derive(Debug, Clone)]
pub struct ConsensusState {
    /// Consensus log index (median of peer log indexes)
    pub log_index: u64,

    /// Consensus vector clock (max of all peer clocks)
    pub vector_clock: VectorClock,
}

/// Active catch-up session
#[derive(Debug, Clone)]
pub struct CatchUpSession {
    /// Unique session ID
    pub session_id: String,

    /// Source node ID
    pub source_node_id: String,

    /// Snapshot vector clock from source
    pub snapshot_vector_clock: VectorClock,
}

/// Result of checkpoint transfer
#[derive(Debug, Clone)]
pub struct CheckpointTransferResult {
    /// Number of SST files transferred
    pub num_files: usize,

    /// Total bytes transferred
    pub total_bytes: u64,

    /// Transfer duration
    pub duration: Duration,
}

/// Result of index transfer
#[derive(Debug, Clone)]
pub struct IndexTransferResult {
    /// Number of Tantivy index files transferred
    pub tantivy_files: usize,

    /// Number of HNSW indexes transferred
    pub hnsw_indexes: usize,
}

/// Result of log verification
#[derive(Debug, Clone)]
pub struct VerificationResult {
    /// Number of operations applied
    pub operations_applied: usize,

    /// Number of conflicts resolved
    pub conflicts_resolved: usize,
}

/// Overall catch-up result
#[derive(Debug, Clone)]
pub struct CatchUpResult {
    /// Source peer ID
    pub source_peer_id: String,

    /// Checkpoint transfer result
    pub checkpoint_result: CheckpointTransferResult,

    /// Index transfer result
    pub index_result: IndexTransferResult,

    /// Verification result
    pub verification_result: VerificationResult,
}
