//! Deletion operations for the operation log

use super::helpers::{build_operation_key, get_oplog_cf};
use super::OpLogRepository;
use raisin_error::{Error, Result};
use raisin_replication::Operation;
use rocksdb::WriteBatch;

impl OpLogRepository {
    /// Delete operations older than a certain age (for garbage collection)
    ///
    /// Returns the number of operations deleted
    pub fn delete_operations_older_than(
        &self,
        tenant_id: &str,
        repo_id: &str,
        age_days: u64,
    ) -> Result<usize> {
        let cutoff_timestamp =
            chrono::Utc::now().timestamp_millis() as u64 - (age_days * 24 * 60 * 60 * 1000);

        let all_ops = self.get_all_operations(tenant_id, repo_id)?;
        let cf = get_oplog_cf(&self.db)?;

        let mut batch = WriteBatch::default();
        let mut deleted_count = 0;

        for (_, ops) in all_ops {
            for op in ops {
                if op.timestamp_ms < cutoff_timestamp {
                    let key = build_operation_key(&op);
                    batch.delete_cf(&cf, &key);
                    deleted_count += 1;
                }
            }
        }

        if deleted_count > 0 {
            self.db
                .write(batch)
                .map_err(|e| Error::storage(format!("Failed to delete operations: {}", e)))?;
        }

        Ok(deleted_count)
    }

    /// Delete a specific operation
    pub fn delete_operation(&self, op: &Operation) -> Result<()> {
        let cf = get_oplog_cf(&self.db)?;
        let key = build_operation_key(op);

        self.db
            .delete_cf(&cf, &key)
            .map_err(|e| Error::storage(format!("Failed to delete operation: {}", e)))?;

        Ok(())
    }

    /// Delete operations by their IDs (used by GC)
    ///
    /// This is a lower-level method for deleting specific operations.
    pub fn delete_operations_by_ids(
        &self,
        tenant_id: &str,
        repo_id: &str,
        op_ids: &[uuid::Uuid],
    ) -> Result<usize> {
        if op_ids.is_empty() {
            return Ok(0);
        }

        // Get all operations to find matching IDs
        let all_ops_map = self.get_all_operations(tenant_id, repo_id)?;
        let cf = get_oplog_cf(&self.db)?;
        let mut batch = WriteBatch::default();
        let mut deleted_count = 0;

        for (_, ops) in all_ops_map {
            for op in ops {
                if op_ids.contains(&op.op_id) {
                    let key = build_operation_key(&op);
                    batch.delete_cf(&cf, &key);
                    deleted_count += 1;
                }
            }
        }

        if deleted_count > 0 {
            self.db
                .write(batch)
                .map_err(|e| Error::storage(format!("Failed to delete operations: {}", e)))?;
        }

        Ok(deleted_count)
    }
}
