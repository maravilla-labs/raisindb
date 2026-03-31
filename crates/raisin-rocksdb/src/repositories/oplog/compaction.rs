//! Operation log compaction logic

use super::helpers::{build_operation_key, get_oplog_cf, serialize_operation};
use super::OpLogRepository;
use hashbrown::HashMap;
use raisin_error::{Error, Result};
use raisin_replication::Operation;
use rocksdb::WriteBatch;

impl OpLogRepository {
    /// Compact operation log for a tenant/repo
    ///
    /// This reduces the size of the operation log by merging redundant operations
    /// while preserving CRDT semantics. Operations are compacted per cluster node.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `compactor` - Compactor instance with configuration
    ///
    /// # Returns
    ///
    /// A map of cluster node IDs to their compaction results
    pub fn compact_oplog(
        &self,
        tenant_id: &str,
        repo_id: &str,
        compactor: &raisin_replication::OperationLogCompactor,
    ) -> Result<HashMap<String, raisin_replication::CompactionResult>> {
        // Get all operations grouped by cluster node
        let all_ops = self.get_all_operations(tenant_id, repo_id)?;
        let current_time = chrono::Utc::now().timestamp_millis() as u64;

        let mut results = HashMap::new();

        for (cluster_node_id, operations) in all_ops {
            // Compact operations for this cluster node
            let (compacted_ops, result) =
                compactor.compact_node_operations(operations.clone(), current_time);

            // If compaction reduced operations, update the log
            if result.merged_count > 0 {
                self.replace_node_operations(
                    tenant_id,
                    repo_id,
                    &cluster_node_id,
                    &operations,
                    &compacted_ops,
                )?;
            }

            results.insert(cluster_node_id, result);
        }

        Ok(results)
    }

    /// Replace operations for a specific cluster node
    ///
    /// This atomically deletes old operations and writes compacted operations.
    ///
    /// # Safety
    ///
    /// This operation must be atomic to prevent data loss. Uses RocksDB WriteBatch.
    fn replace_node_operations(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        cluster_node_id: &str,
        old_ops: &[Operation],
        new_ops: &[Operation],
    ) -> Result<()> {
        let cf = get_oplog_cf(&self.db)?;
        let mut batch = WriteBatch::default();

        // Delete all old operations
        for op in old_ops {
            let key = build_operation_key(op);
            batch.delete_cf(&cf, &key);
        }

        // Write compacted operations
        for op in new_ops {
            let key = build_operation_key(op);
            let value = serialize_operation(op)?;
            batch.put_cf(&cf, &key, &value);
        }

        // Atomic write
        self.db.write(batch).map_err(|e| {
            Error::storage(format!(
                "Failed to replace operations for node {}: {}",
                cluster_node_id, e
            ))
        })?;

        Ok(())
    }
}
