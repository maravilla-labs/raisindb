// TODO(v0.2): Clean up unused code
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]

//! # RaisinDB Replication
//!
//! Operation-based CRDT replication system for RaisinDB clustering and offline sync.
//!
//! ## Architecture
//!
//! This crate provides the core primitives for masterless multi-master replication:
//!
//! - **Vector Clocks**: Track causal dependencies between operations
//! - **Operations**: Replayable, idempotent mutations
//! - **CRDT Merge Rules**: Conflict-free convergence algorithms
//! - **Operation Log**: Durable log of all mutations
//! - **Replay Engine**: Apply operations in causal order
//! - **Garbage Collection**: Bounded growth for operation log
//!
//! ## Key Features
//!
//! - **Masterless**: No single point of failure, any node can accept writes
//! - **Causal Consistency**: Operations applied in happens-before order
//! - **Eventual Consistency**: All nodes converge to identical state
//! - **Conflict-Free**: Deterministic merge rules for concurrent operations
//! - **Offline-First**: Operations queue locally and sync later
//!
//! ## CRDT Merge Rules
//!
//! Different operation types use different CRDTs:
//!
//! - **Properties**: Last-Write-Wins with vector clock + timestamp + node_id tie-breaking
//! - **Relations/Sets**: Add-Wins Set CRDT (additions beat deletions)
//! - **Ordered Lists**: RGA (Replicated Growable Array) with tombstones
//! - **Moves**: Last-Write-Wins with conflict event emission
//! - **Deletes**: Delete-Wins (prevents resurrection)
//!
//! ## Example Usage
//!
//! ```rust,ignore
//! use raisin_replication::{VectorClock, Operation, OpType, CrdtMerge};
//!
//! // Create a vector clock and increment for local operation
//! let mut vc = VectorClock::new();
//! vc.increment("node1");
//!
//! // Create an operation
//! let op = Operation::new(
//!     1,                    // op_seq
//!     "node1".to_string(),  // node_id
//!     vc,                   // vector_clock
//!     "tenant1".to_string(),
//!     "repo1".to_string(),
//!     "main".to_string(),
//!     OpType::SetProperty {
//!         node_id: "abc123".to_string(),
//!         property_name: "title".to_string(),
//!         value: serde_json::json!("Hello World"),
//!     },
//!     "user@example.com".to_string(),
//! );
//!
//! // Merge concurrent operations using CRDT rules
//! let result = CrdtMerge::merge_operations(vec![op1, op2]);
//! ```

pub mod catch_up;
pub mod causal_delivery;
pub mod compaction;
pub mod config;
pub mod conflict_resolution;
pub mod coordinator;
pub mod crdt;
pub mod gc;
pub mod metrics;
pub mod metrics_reporter;
pub mod operation;
pub mod operation_decomposer;
pub mod operation_decomposer_metrics;
pub mod peer_manager;
pub mod priority;
pub mod replay;
pub mod streaming;
pub mod tcp_helpers;
pub mod tcp_protocol;
pub mod tcp_server;
pub mod value_conversion;
pub mod vector_clock;

// Re-export commonly used types
pub use catch_up::{
    CatchUpCoordinator, CatchUpResult, CatchUpSession, CheckpointTransferResult, ConsensusState,
    IndexTransferResult, PeerStatus as CatchUpPeerStatus, VerificationResult,
};
pub use causal_delivery::{BufferStats as CausalBufferStats, CausalDeliveryBuffer};
pub use compaction::{
    CompactionConfig, CompactionResult, NodeCompactionStats, OperationLogCompactor,
};
pub use config::{ClusterConfig, ConnectionConfig, PeerConfig, RetryConfig, SyncConfig};
pub use conflict_resolution::{ConflictError, ConflictGroup, ConflictResolver};
pub use coordinator::{
    CheckpointIngestor, CheckpointProvider, ClusterStorageStats, CoordinatorError,
    HnswIndexReceiver, IndexLister, OperationLogStorage, ReplicationCoordinator, StorageError,
    SyncStats, TantivyIndexReceiver,
};
pub use crdt::{ConflictType, CrdtMerge, MergeResult};
pub use gc::{GarbageCollector, GcConfig, GcResult, GcStrategy, PeerWatermarks};
pub use metrics::{
    AggregateMetrics, CausalBufferMetrics, DecompositionMetrics, IdempotencyMetrics,
    ReplicationMetrics,
};
pub use metrics_reporter::{metrics_to_json, metrics_to_json_compact, MetricsReporter};
pub use operation::{OpType, Operation, OperationTarget};
pub use operation_decomposer::decompose_operation;
pub use operation_decomposer_metrics::OperationDecomposer;
pub use peer_manager::{ConnectionState, PeerManager, PeerManagerError, PeerStatus};
pub use priority::{sort_operations_by_priority, OperationPriority};
pub use replay::{
    ConflictInfo, IdempotencyTracker, InMemoryIdempotencyTracker, ReplayEngine, ReplayResult,
};
pub use streaming::{
    ChunkAck, FileChunk, FileInfo, ParallelTransferOrchestrator, ReliableFileStreamer, StreamError,
    DEFAULT_CHUNK_SIZE, TANTIVY_CHUNK_SIZE,
};
pub use tcp_protocol::{
    ErrorCode, IndexFileInfo, ProtocolError, ReplicationMessage, SstFileInfo, TransferStatus,
    PROTOCOL_VERSION,
};
pub use tcp_server::ReplicationServer;
pub use value_conversion::{json_to_msgpack, msgpack_to_json};
pub use vector_clock::{ClockOrdering, VectorClock};
