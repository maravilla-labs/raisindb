//! Replication message enum definition
//!
//! NOTE: This file intentionally exceeds 300 lines because the `ReplicationMessage`
//! enum has 30+ variants (each with documented fields) that form a single cohesive
//! type. Splitting the enum across files is not possible in Rust.
//!
//! See also: [`message_impl`](super::message_impl) for serialization, deserialization,
//! wire encoding, and convenience constructors.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Operation, VectorClock};

use super::constants::{default_batch_size, default_max_parallel_files};
use super::error::ErrorCode;
use super::file_transfer::{IndexFileInfo, SstFileInfo, TransferStatus};

/// Messages exchanged over TCP between replication peers
///
/// Each message is prefixed with a 4-byte length header (big-endian u32)
/// followed by the MessagePack-encoded message body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplicationMessage {
    /// Initial handshake from client to server
    ///
    /// Establishes connection and verifies protocol version compatibility.
    Hello {
        /// Unique identifier for the sending cluster node
        cluster_node_id: String,

        /// Protocol version the client supports
        protocol_version: u8,

        /// Optional metadata (e.g., node capabilities, region)
        #[serde(default)]
        metadata: Option<serde_json::Value>,
    },

    /// Acknowledgment of Hello from server
    HelloAck {
        /// Server's cluster node ID
        cluster_node_id: String,

        /// Protocol version the server will use (negotiated)
        protocol_version: u8,

        /// Server metadata
        #[serde(default)]
        metadata: Option<serde_json::Value>,
    },

    /// Request current vector clock for a repository
    ///
    /// Used to determine which operations are missing.
    GetVectorClock {
        /// Tenant identifier
        tenant_id: String,

        /// Repository identifier
        repo_id: String,
    },

    /// Response containing current vector clock
    VectorClockResponse {
        /// Tenant identifier
        tenant_id: String,

        /// Repository identifier
        repo_id: String,

        /// Current vector clock snapshot
        vector_clock: VectorClock,
    },

    /// Request to pull operations from peer
    ///
    /// The peer will respond with operations that are newer than the
    /// provided vector clock.
    PullOperations {
        /// Tenant identifier
        tenant_id: String,

        /// Repository identifier
        repo_id: String,

        /// Client's current vector clock
        since_vector_clock: VectorClock,

        /// Optional filter to only sync specific branches
        #[serde(default)]
        branch_filter: Option<Vec<String>>,

        /// Maximum number of operations to return
        #[serde(default = "default_batch_size")]
        limit: usize,
    },

    /// Push operations to peer
    ///
    /// Used for real-time replication when new operations are committed.
    PushOperations {
        /// Operations to replicate
        operations: Vec<Operation>,
    },

    /// Response containing a batch of operations
    OperationBatch {
        /// Operations being sent
        operations: Vec<Operation>,

        /// Whether more operations are available
        has_more: bool,

        /// Total number of operations available (for progress tracking)
        total_available: usize,
    },

    /// Acknowledge receipt of operations
    ///
    /// Used for tracking which operations have been successfully replicated
    /// for garbage collection purposes.
    Ack {
        /// Operation IDs that were successfully applied
        op_ids: Vec<Uuid>,
    },

    /// Heartbeat ping to keep connection alive
    Ping {
        /// Timestamp when ping was sent (milliseconds since epoch)
        timestamp_ms: u64,
    },

    /// Response to heartbeat ping
    Pong {
        /// Echo of the timestamp from Ping
        timestamp_ms: u64,
    },

    /// Error response
    Error {
        /// Error code
        code: ErrorCode,

        /// Human-readable error message
        message: String,

        /// Optional additional details
        #[serde(default)]
        details: Option<serde_json::Value>,
    },

    // === Cluster Catch-Up Protocol Messages ===
    /// Request cluster status from peer
    ///
    /// Used during catch-up to discover cluster topology and determine
    /// which peer has the most up-to-date state.
    GetClusterStatus,

    /// Response with cluster status information
    ClusterStatusResponse {
        /// Responding node's ID
        node_id: String,

        /// Current operation log index
        log_index: u64,

        /// Current maximum vector clock for this node
        max_vector_clock: VectorClock,

        /// Number of tenants on this node
        num_tenants: usize,

        /// Number of repositories on this node
        num_repos: usize,

        /// Timestamp of last update (ms since epoch)
        last_update_timestamp_ms: u64,

        /// List of peer IDs this node knows about
        known_peers: Vec<String>,

        /// Detailed list of tenant/repository pairs hosted on this node
        #[serde(default)]
        tenant_repos: Vec<(String, String)>,

        /// Total storage size in bytes (optional, for planning)
        #[serde(default)]
        storage_size_bytes: Option<u64>,
    },

    /// Initiate catch-up process with a source node
    InitiateCatchUp {
        /// Node requesting catch-up
        requesting_node: String,

        /// Requester's current vector clock
        local_vector_clock: VectorClock,
    },

    /// Acknowledgment of catch-up initiation
    CatchUpAck {
        /// Source node providing the snapshot
        source_node: String,

        /// Unique snapshot ID for this catch-up session
        snapshot_id: String,

        /// Vector clock at snapshot time
        snapshot_vector_clock: VectorClock,

        /// Estimated total transfer size in bytes
        #[serde(default)]
        estimated_transfer_size_bytes: Option<u64>,
    },

    /// Request RocksDB checkpoint metadata
    RequestCheckpoint {
        /// Snapshot ID from CatchUpAck
        snapshot_id: String,

        /// Maximum number of files to transfer in parallel
        #[serde(default = "default_max_parallel_files")]
        max_parallel_files: u8,
    },

    /// Checkpoint metadata listing all SST files
    CheckpointMetadata {
        /// Snapshot ID
        snapshot_id: String,

        /// List of SST files to transfer
        sst_files: Vec<SstFileInfo>,

        /// Total size of all files in bytes
        total_size_bytes: u64,

        /// Column families included
        column_families: Vec<String>,
    },

    /// SST file chunk with checksum
    SstFileChunk {
        /// Snapshot ID
        snapshot_id: String,

        /// File name being transferred
        file_name: String,

        /// Chunk index (0-based)
        chunk_index: u32,

        /// Total number of chunks for this file
        total_chunks: u32,

        /// Chunk data
        data: Vec<u8>,

        /// CRC32 checksum of this chunk
        chunk_crc32: u32,
    },

    /// Acknowledgment of SST file chunk receipt
    SstFileChunkAck {
        /// File name
        file_name: String,

        /// Chunk index
        chunk_index: u32,

        /// Transfer status
        status: TransferStatus,
    },

    /// Request list of available Tantivy indexes
    RequestTantivyIndexList,

    /// Response with list of Tantivy indexes
    TantivyIndexList {
        /// List of (tenant_id, repo_id, branch) tuples
        indexes: Vec<(String, String, String)>,
    },

    /// Request Tantivy fulltext index
    RequestTantivyIndex {
        /// Tenant ID
        tenant_id: String,

        /// Repository ID
        repo_id: String,

        /// Branch name
        branch: String,
    },

    /// Tantivy index metadata
    TantivyIndexMetadata {
        /// Tenant ID
        tenant_id: String,

        /// Repository ID
        repo_id: String,

        /// Branch name
        branch: String,

        /// List of index files
        files: Vec<IndexFileInfo>,

        /// Total size in bytes
        total_size_bytes: u64,
    },

    /// Tantivy file chunk with checksum
    TantivyFileChunk {
        /// Tenant ID
        tenant_id: String,

        /// Repository ID
        repo_id: String,

        /// Branch name
        branch: String,

        /// File name
        file_name: String,

        /// Chunk index
        chunk_index: u32,

        /// Total chunks
        total_chunks: u32,

        /// Chunk data
        data: Vec<u8>,

        /// CRC32 checksum
        chunk_crc32: u32,
    },

    /// Tantivy file chunk acknowledgment
    TantivyFileChunkAck {
        /// File name
        file_name: String,

        /// Chunk index
        chunk_index: u32,

        /// Transfer status
        status: TransferStatus,
    },

    /// Request list of available HNSW indexes
    RequestHnswIndexList,

    /// Response with list of HNSW indexes
    HnswIndexList {
        /// List of (tenant_id, repo_id, branch) tuples
        indexes: Vec<(String, String, String)>,
    },

    /// Request HNSW vector index
    RequestHnswIndex {
        /// Tenant ID
        tenant_id: String,

        /// Repository ID
        repo_id: String,

        /// Branch name
        branch: String,
    },

    /// HNSW index data (entire file, typically small)
    HnswIndexData {
        /// Tenant ID
        tenant_id: String,

        /// Repository ID
        repo_id: String,

        /// Branch name
        branch: String,

        /// Complete .hnsw file data
        data: Vec<u8>,

        /// CRC32 checksum of entire file
        crc32: u32,
    },

    /// HNSW index acknowledgment
    HnswIndexAck {
        /// Transfer status
        status: TransferStatus,
    },

    /// Request operation log tail for verification
    RequestLogTail {
        /// Start from this vector clock
        since_vector_clock: VectorClock,

        /// Maximum operations to return
        max_operations: u32,
    },

    /// Response with log tail operations
    LogTailResponse {
        /// Operations since vector clock
        operations: Vec<Operation>,

        /// Peer's current vector clock
        peer_vector_clock: VectorClock,

        /// Whether more operations are available
        has_more: bool,
    },

    /// Signal verification complete
    VerificationComplete {
        /// Final merged vector clock after verification
        final_vector_clock: VectorClock,
    },

    /// Announce node is ready for full participation
    NodeReady {
        /// Node ID
        node_id: String,

        /// Current vector clock
        vector_clock: VectorClock,
    },
}
