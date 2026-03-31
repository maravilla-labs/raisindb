//! Garbage collection operations for the operation log

use super::helpers::{build_operation_key, get_oplog_cf, serialize_operation};
use super::OpLogRepository;
use hashbrown::HashMap;
use raisin_error::{Error, Result};
use rocksdb::WriteBatch;

impl OpLogRepository {
    /// Perform garbage collection using the GarbageCollector
    ///
    /// This is the main GC method that integrates the GC module.
    /// Returns the number of operations deleted and bytes reclaimed.
    pub fn garbage_collect(
        &self,
        tenant_id: &str,
        repo_id: &str,
        gc: &raisin_replication::GarbageCollector,
    ) -> Result<raisin_replication::GcResult> {
        use raisin_replication::GcStrategy;

        // Get all operations for this tenant/repo
        let all_ops_map = self.get_all_operations(tenant_id, repo_id)?;
        let mut all_ops = Vec::new();
        for (_, ops) in all_ops_map {
            all_ops.extend(ops);
        }

        // Get current log size
        let stats = self.get_stats(tenant_id, repo_id)?;
        let estimated_size = stats.total_operations * 1024; // Rough estimate: 1KB per op

        // Determine which operations to delete
        let (to_delete_ids, strategy) = gc.collect(&all_ops, estimated_size as u64);

        if to_delete_ids.is_empty() {
            return Ok(raisin_replication::GcResult {
                deleted_count: 0,
                bytes_reclaimed: 0,
                strategy: GcStrategy::NoOp,
                deleted_by_node: HashMap::new(),
                watermarks: gc.watermarks().clone(),
            });
        }

        // Delete operations in batch
        let cf = get_oplog_cf(&self.db)?;
        let mut batch = WriteBatch::default();
        let mut deleted_by_node: HashMap<String, usize> = HashMap::new();
        let mut bytes_reclaimed = 0u64;

        for op in &all_ops {
            if to_delete_ids.contains(&op.op_id) {
                let key = build_operation_key(op);
                batch.delete_cf(&cf, &key);

                // Track per-node deletions
                *deleted_by_node
                    .entry(op.cluster_node_id.clone())
                    .or_insert(0) += 1;

                // Estimate bytes reclaimed (MessagePack serialized size)
                bytes_reclaimed += serialize_operation(op)
                    .map(|v| v.len() as u64)
                    .unwrap_or(1024);
            }
        }

        // Commit batch
        self.db
            .write(batch)
            .map_err(|e| Error::storage(format!("Failed to execute GC batch: {}", e)))?;

        Ok(raisin_replication::GcResult {
            deleted_count: to_delete_ids.len(),
            bytes_reclaimed,
            strategy,
            deleted_by_node,
            watermarks: gc.watermarks().clone(),
        })
    }
}
