//! Replication and maintenance job handler construction
//!
//! Creates handlers for snapshots, replication GC, replication sync,
//! and oplog compaction.

use std::sync::Arc;

use crate::jobs::{OpLogCompactionHandler, ReplicationGCHandler};
use crate::storage::RocksDBStorage;

/// Create the snapshot handler
pub fn create_snapshot_handler(storage: &RocksDBStorage) -> Arc<crate::jobs::SnapshotHandler> {
    Arc::new(crate::jobs::SnapshotHandler::new(storage.db.clone()))
}

/// Create the replication GC handler
pub fn create_replication_gc_handler(storage: &RocksDBStorage) -> Arc<ReplicationGCHandler> {
    Arc::new(ReplicationGCHandler::new(storage.db.clone()))
}

/// Create the replication sync handler
pub fn create_replication_sync_handler(
    storage: &RocksDBStorage,
) -> Arc<crate::jobs::ReplicationSyncHandler> {
    let cluster_node_id = storage
        .config
        .cluster_node_id
        .clone()
        .unwrap_or_else(|| nanoid::nanoid!(16));

    Arc::new(crate::jobs::ReplicationSyncHandler::new(
        storage.db.clone(),
        cluster_node_id,
    ))
}

/// Create the oplog compaction handler
pub fn create_oplog_compaction_handler(storage: &RocksDBStorage) -> Arc<OpLogCompactionHandler> {
    let compaction_config = raisin_replication::CompactionConfig {
        min_age_secs: storage.config.oplog_compaction_min_age_secs,
        merge_property_updates: storage.config.oplog_merge_property_updates,
        batch_size: storage.config.oplog_compaction_batch_size,
    };

    Arc::new(OpLogCompactionHandler::with_config(
        storage.db.clone(),
        compaction_config,
    ))
}
