//! Query operations for the operation log

use super::helpers::{deserialize_operation, get_oplog_cf};
use super::types::OpLogStats;
use super::OpLogRepository;
use crate::keys::{
    oplog_from_seq_prefix, oplog_node_prefix, oplog_tenant_repo_prefix, vector_clock_snapshot_key,
};
use hashbrown::HashMap;
use raisin_error::{Error, Result};
use raisin_replication::{Operation, VectorClock};
use rocksdb::{Direction, IteratorMode};

impl OpLogRepository {
    /// Get all operations from a specific node starting from a sequence number
    ///
    /// Returns operations in ascending order (oldest first)
    pub fn get_operations_from_seq(
        &self,
        tenant_id: &str,
        repo_id: &str,
        node_id: &str,
        from_seq: u64,
    ) -> Result<Vec<Operation>> {
        let cf = get_oplog_cf(&self.db)?;
        let prefix = oplog_from_seq_prefix(tenant_id, repo_id, node_id, from_seq);

        let mut operations = Vec::new();
        let iter = self
            .db
            .iterator_cf(&cf, IteratorMode::From(&prefix, Direction::Forward));

        for item in iter {
            let (key, value) =
                item.map_err(|e| Error::storage(format!("Iterator error: {}", e)))?;

            // Check if still within our prefix
            if !key.starts_with(&prefix[..prefix.len() - 9]) {
                // -9 to account for the sequence bytes + separator
                break;
            }

            let op = deserialize_operation(
                &value,
                Some("get_operations_from_seq"),
                Some(tenant_id),
                Some(repo_id),
            )?;
            operations.push(op);
        }

        Ok(operations)
    }

    /// Get all operations from a specific node
    ///
    /// Returns operations in ascending order (oldest first)
    pub fn get_operations_from_node(
        &self,
        tenant_id: &str,
        repo_id: &str,
        node_id: &str,
    ) -> Result<Vec<Operation>> {
        let cf = get_oplog_cf(&self.db)?;
        let prefix = oplog_node_prefix(tenant_id, repo_id, node_id);

        let mut operations = Vec::new();
        let iter = self
            .db
            .iterator_cf(&cf, IteratorMode::From(&prefix, Direction::Forward));

        for item in iter {
            let (key, value) =
                item.map_err(|e| Error::storage(format!("Iterator error: {}", e)))?;

            // Check if still within our prefix
            if !key.starts_with(&prefix[..prefix.len() - 1]) {
                break;
            }

            let op = deserialize_operation(
                &value,
                Some("get_operations_from_node"),
                Some(tenant_id),
                Some(repo_id),
            )?;
            operations.push(op);
        }

        Ok(operations)
    }

    /// Get all operations for a tenant/repo across all nodes
    ///
    /// Returns operations grouped by node_id
    pub fn get_all_operations(
        &self,
        tenant_id: &str,
        repo_id: &str,
    ) -> Result<HashMap<String, Vec<Operation>>> {
        let cf = get_oplog_cf(&self.db)?;
        let prefix = oplog_tenant_repo_prefix(tenant_id, repo_id);
        let snapshot_key = vector_clock_snapshot_key(tenant_id, repo_id);

        let mut operations_by_node: HashMap<String, Vec<Operation>> = HashMap::new();
        let iter = self
            .db
            .iterator_cf(&cf, IteratorMode::From(&prefix, Direction::Forward));

        for item in iter {
            let (key, value) =
                item.map_err(|e| Error::storage(format!("Iterator error: {}", e)))?;

            // Check if still within our prefix
            if !key.starts_with(&prefix[..prefix.len() - 1]) {
                break;
            }

            // Skip vector clock snapshot entries that share the same prefix
            if key.as_ref() == snapshot_key.as_slice() {
                continue;
            }

            let op = deserialize_operation(
                &value,
                Some("get_all_operations"),
                Some(tenant_id),
                Some(repo_id),
            )?;

            operations_by_node
                .entry(op.cluster_node_id.clone())
                .or_insert_with(Vec::new)
                .push(op);
        }

        Ok(operations_by_node)
    }

    /// Get operations that are missing based on vector clock
    ///
    /// Returns operations that are NOT reflected in the provided vector clock.
    /// This is used for pull-based synchronization.
    ///
    /// # Arguments
    /// * `limit` - Maximum number of operations to return (prevents OOM on large backlogs)
    ///
    /// # Memory Safety
    /// This method streams operations instead of loading all into memory,
    /// and respects the limit to prevent out-of-memory errors.
    pub fn get_missing_operations(
        &self,
        tenant_id: &str,
        repo_id: &str,
        vector_clock: &VectorClock,
        limit: Option<usize>,
    ) -> Result<Vec<Operation>> {
        let cf = get_oplog_cf(&self.db)?;
        let prefix = oplog_tenant_repo_prefix(tenant_id, repo_id);
        let snapshot_key = vector_clock_snapshot_key(tenant_id, repo_id);

        let mut missing = Vec::new();
        let max_operations = limit.unwrap_or(usize::MAX);

        let iter = self
            .db
            .iterator_cf(&cf, IteratorMode::From(&prefix, Direction::Forward));

        for item in iter {
            // Early exit if we've reached the limit
            if missing.len() >= max_operations {
                break;
            }

            let (key, value) =
                item.map_err(|e| Error::storage(format!("Iterator error: {}", e)))?;

            // Check if still within our prefix
            if !key.starts_with(&prefix[..prefix.len() - 1]) {
                break;
            }

            // Skip vector clock snapshot entries that share the same prefix
            if key.as_ref() == snapshot_key.as_slice() {
                continue;
            }

            let op = deserialize_operation(
                &value,
                Some("get_missing_operations"),
                Some(tenant_id),
                Some(repo_id),
            )?;

            // Check if this operation is missing from the vector clock
            let last_seen_seq = vector_clock.get(&op.cluster_node_id);
            if op.op_seq > last_seen_seq {
                missing.push(op);
            }
        }

        Ok(missing)
    }

    /// Get operations that are missing based on vector clock (deprecated - unbounded)
    ///
    /// **WARNING**: This method loads ALL operations into memory and should not be used
    /// for production. Use `get_missing_operations` with a limit instead.
    #[deprecated(note = "Use get_missing_operations with limit parameter to prevent OOM")]
    #[allow(dead_code)]
    pub fn get_missing_operations_unbounded(
        &self,
        tenant_id: &str,
        repo_id: &str,
        vector_clock: &VectorClock,
    ) -> Result<Vec<Operation>> {
        let all_ops = self.get_all_operations(tenant_id, repo_id)?;
        let mut missing = Vec::new();

        for (node_id, ops) in all_ops {
            let last_seen_seq = vector_clock.get(&node_id);

            // Find operations with sequence > last_seen
            for op in ops {
                if op.op_seq > last_seen_seq {
                    missing.push(op);
                }
            }
        }

        Ok(missing)
    }

    /// Get the highest sequence number for a specific node
    ///
    /// Returns 0 if no operations exist
    pub fn get_highest_seq(&self, tenant_id: &str, repo_id: &str, node_id: &str) -> Result<u64> {
        let cf = get_oplog_cf(&self.db)?;
        let prefix = oplog_node_prefix(tenant_id, repo_id, node_id);

        // Iterate backwards from the prefix to get the latest
        let iter = self
            .db
            .iterator_cf(&cf, IteratorMode::From(&prefix, Direction::Forward));

        let mut highest = 0u64;

        for item in iter {
            let (key, value) =
                item.map_err(|e| Error::storage(format!("Iterator error: {}", e)))?;

            // Check if still within our prefix
            if !key.starts_with(&prefix[..prefix.len() - 1]) {
                break;
            }

            let op: Operation = rmp_serde::from_slice(&value)
                .map_err(|e| Error::storage(format!("Failed to deserialize operation: {}", e)))?;

            if op.op_seq > highest {
                highest = op.op_seq;
            }
        }

        Ok(highest)
    }

    /// Get statistics about the operation log
    pub fn get_stats(&self, tenant_id: &str, repo_id: &str) -> Result<OpLogStats> {
        let all_ops = self.get_all_operations(tenant_id, repo_id)?;

        let mut stats = OpLogStats {
            total_operations: 0,
            operations_per_node: HashMap::new(),
            oldest_operation_timestamp: None,
            newest_operation_timestamp: None,
        };

        for (node_id, ops) in all_ops {
            stats.total_operations += ops.len();
            stats.operations_per_node.insert(node_id.clone(), ops.len());

            for op in ops {
                // Note: total_operations already counted above via ops.len()
                // Update timestamp bounds
                match stats.oldest_operation_timestamp {
                    None => stats.oldest_operation_timestamp = Some(op.timestamp_ms),
                    Some(oldest) if op.timestamp_ms < oldest => {
                        stats.oldest_operation_timestamp = Some(op.timestamp_ms);
                    }
                    _ => {}
                }

                match stats.newest_operation_timestamp {
                    None => stats.newest_operation_timestamp = Some(op.timestamp_ms),
                    Some(newest) if op.timestamp_ms > newest => {
                        stats.newest_operation_timestamp = Some(op.timestamp_ms);
                    }
                    _ => {}
                }
            }
        }

        Ok(stats)
    }
}
