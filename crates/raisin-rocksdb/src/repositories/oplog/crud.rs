//! Basic CRUD operations for the operation log

use super::helpers::{build_operation_key, get_oplog_cf, serialize_operation, write_batch};
use super::OpLogRepository;
use hashbrown::HashMap;
use raisin_error::Result;
use raisin_replication::Operation;
use rocksdb::WriteBatch;

impl OpLogRepository {
    /// Store an operation in the operation log
    pub fn put_operation(&self, op: &Operation) -> Result<()> {
        let cf = get_oplog_cf(&self.db)?;
        let key = build_operation_key(op);
        let value = serialize_operation(op)?;

        self.db.put_cf(&cf, &key, &value).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to store operation: {}", e))
        })?;

        Ok(())
    }

    /// Store multiple operations in a batch
    ///
    /// This method also updates the vector clock snapshot for each tenant/repo
    /// affected by the batch to maintain snapshot consistency.
    pub fn put_operations_batch(&self, ops: &[Operation]) -> Result<()> {
        if ops.is_empty() {
            return Ok(());
        }

        let cf = get_oplog_cf(&self.db)?;
        let mut batch = WriteBatch::default();

        // Track the highest op_seq per (tenant, repo, cluster_node)
        let mut updates: HashMap<(String, String), HashMap<String, u64>> = HashMap::new();

        for op in ops {
            let key = build_operation_key(op);
            let value = serialize_operation(op)?;
            batch.put_cf(&cf, &key, &value);

            // Track max op_seq for vector clock snapshot update
            let tenant_repo_key = (op.tenant_id.clone(), op.repo_id.clone());
            let node_max = updates.entry(tenant_repo_key).or_default();
            let current = node_max.get(&op.cluster_node_id).copied().unwrap_or(0);
            if op.op_seq > current {
                node_max.insert(op.cluster_node_id.clone(), op.op_seq);
            }
        }

        // Write operations batch first
        write_batch(&self.db, batch)?;

        // Update vector clock snapshots for each affected tenant/repo
        for ((tenant_id, repo_id), node_updates) in updates {
            for (cluster_node_id, op_seq) in node_updates {
                self.increment_vector_clock_for_node(
                    &tenant_id,
                    &repo_id,
                    &cluster_node_id,
                    op_seq,
                )?;
            }
        }

        Ok(())
    }

    /// Update acknowledgments for operations
    ///
    /// This marks operations as acknowledged by a peer, which is used for GC.
    pub fn acknowledge_operations(
        &self,
        tenant_id: &str,
        repo_id: &str,
        node_id: &str,
        peer_id: String,
        up_to_seq: u64,
    ) -> Result<usize> {
        let ops = self.get_operations_from_node(tenant_id, repo_id, node_id)?;
        let cf = get_oplog_cf(&self.db)?;
        let mut batch = WriteBatch::default();
        let mut updated_count = 0;

        for mut op in ops {
            if op.op_seq <= up_to_seq {
                // Add peer to acknowledged_by set
                if !op.acknowledged_by.contains(&peer_id) {
                    op.acknowledged_by.insert(peer_id.clone());

                    // Re-serialize and write back
                    let key = build_operation_key(&op);
                    let value = serialize_operation(&op)?;
                    batch.put_cf(&cf, &key, &value);
                    updated_count += 1;
                }
            }
        }

        if updated_count > 0 {
            self.db.write(batch).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to acknowledge operations: {}", e))
            })?;
        }

        Ok(updated_count)
    }
}
