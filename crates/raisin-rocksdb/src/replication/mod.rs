//! Replication support for RocksDB storage
//!
//! This module provides operation capture and replay for CRDT-based replication.

pub mod application;
pub mod change_tracker;
pub mod checkpoint_server;
pub mod integration;
pub mod operation_capture;
pub mod operation_queue;
pub mod persistent_idempotency;

pub use application::OperationApplicator;
pub use change_tracker::{
    ChangeTracker, NodeChanges, NodeMetadataChanges, NodeMove, PropertyChange, RelationChange,
};
pub use checkpoint_server::CheckpointServer;
pub use integration::{start_replication, RocksDbCheckpointIngestor, RocksDbOperationLogStorage};
pub use operation_capture::OperationCapture;
pub use operation_queue::{OperationQueue, QueueStats, QueueStatsSnapshot, QueuedOperation};
pub use persistent_idempotency::PersistentIdempotencyTracker;
