//! Vector clock snapshot management (Performance Optimization)
//!
//! This module provides operations for managing vector clock snapshots,
//! which represent the highest operation sequence number seen from each cluster node.
//!
//! # Performance Benefits
//!
//! - O(1) constant time lookup (single key read) vs O(n) scanning all operations
//! - 50-5000x faster than rebuilding from operations
//! - Enables efficient pull-based synchronization

use super::helpers::get_oplog_cf;
use super::OpLogRepository;
use crate::keys::{decode_u64, oplog_tenant_repo_prefix, vector_clock_snapshot_key};
use raisin_error::{Error, Result};
use raisin_replication::VectorClock;
use rocksdb::{Direction, IteratorMode};

impl OpLogRepository {
    /// Get the current vector clock snapshot for a tenant/repo
    ///
    /// This retrieves a persisted snapshot of the vector clock, which represents
    /// the highest operation sequence number seen from each cluster node.
    ///
    /// # Performance
    ///
    /// - O(1) constant time lookup (single key read)
    /// - Avoids scanning entire operation log
    /// - 50-5000x faster than rebuilding from operations
    ///
    /// # Returns
    ///
    /// - `Ok(VectorClock)` - The current snapshot, or empty if never initialized
    /// - `Err(_)` - If deserialization fails
    pub fn get_vector_clock_snapshot(&self, tenant_id: &str, repo_id: &str) -> Result<VectorClock> {
        let key = vector_clock_snapshot_key(tenant_id, repo_id);
        let cf = get_oplog_cf(&self.db)?;

        match self
            .db
            .get_cf(&cf, key)
            .map_err(|e| Error::storage(format!("Failed to read vector clock snapshot: {}", e)))?
        {
            Some(bytes) => {
                let vc: VectorClock = rmp_serde::from_slice(&bytes).map_err(|e| {
                    Error::storage(format!(
                        "Failed to deserialize vector clock snapshot: {}",
                        e
                    ))
                })?;
                Ok(vc)
            }
            None => {
                // Return empty vector clock if snapshot doesn't exist yet
                Ok(VectorClock::new())
            }
        }
    }

    /// Update the vector clock snapshot atomically
    ///
    /// This persists a new snapshot of the vector clock. Should be called
    /// after operations are added to ensure the snapshot stays in sync.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `vector_clock` - The new vector clock snapshot to persist
    ///
    /// # Atomicity
    ///
    /// This operation is atomic (single put). For batch updates with operations,
    /// use `increment_vector_clock_for_node` or update snapshot in a WriteBatch.
    pub fn update_vector_clock_snapshot(
        &self,
        tenant_id: &str,
        repo_id: &str,
        vector_clock: &VectorClock,
    ) -> Result<()> {
        let key = vector_clock_snapshot_key(tenant_id, repo_id);
        let cf = get_oplog_cf(&self.db)?;

        let bytes = rmp_serde::to_vec(vector_clock).map_err(|e| {
            Error::storage(format!("Failed to serialize vector clock snapshot: {}", e))
        })?;

        self.db.put_cf(&cf, key, bytes).map_err(|e| {
            Error::storage(format!("Failed to update vector clock snapshot: {}", e))
        })?;

        Ok(())
    }

    /// Incrementally update vector clock snapshot for a single cluster node
    ///
    /// This is the primary method for maintaining the snapshot during normal
    /// operation. It reads the current snapshot, updates the entry for the
    /// specified cluster node if the new op_seq is higher, and persists it back.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `cluster_node_id` - The cluster node that generated the operation
    /// * `op_seq` - The operation sequence number to record
    ///
    /// # Performance
    ///
    /// - O(1) read + O(1) write
    /// - Only updates if op_seq is higher than current value
    /// - Much faster than rebuilding from all operations
    ///
    /// # Example
    ///
    /// ```ignore
    /// // After storing an operation
    /// oplog_repo.put_operation(&op)?;
    /// oplog_repo.increment_vector_clock_for_node(
    ///     &op.tenant_id,
    ///     &op.repo_id,
    ///     &op.cluster_node_id,
    ///     op.op_seq,
    /// )?;
    /// ```
    pub fn increment_vector_clock_for_node(
        &self,
        tenant_id: &str,
        repo_id: &str,
        cluster_node_id: &str,
        op_seq: u64,
    ) -> Result<()> {
        // Get current snapshot
        let mut vc = self.get_vector_clock_snapshot(tenant_id, repo_id)?;

        // Update for this cluster node if new op_seq is higher
        let current = vc.get(cluster_node_id);
        if op_seq > current {
            vc.set(cluster_node_id, op_seq);

            // Persist updated snapshot
            self.update_vector_clock_snapshot(tenant_id, repo_id, &vc)?;
        }

        Ok(())
    }

    /// Rebuild vector clock snapshot from the operation log
    ///
    /// This scans all operations for a tenant/repo and rebuilds the vector clock
    /// from scratch. Use this for:
    /// - Initial snapshot creation on startup
    /// - Verification that snapshot is accurate
    /// - Recovery from corrupted snapshot
    ///
    /// # Performance
    ///
    /// - O(n) where n = total operations
    /// - Can be slow for large operation logs (millions of operations)
    /// - Should not be used during normal operation (use incremental updates)
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    ///
    /// # Returns
    ///
    /// The rebuilt vector clock (also persisted to storage)
    pub fn rebuild_vector_clock_snapshot(
        &self,
        tenant_id: &str,
        repo_id: &str,
    ) -> Result<VectorClock> {
        let cf = get_oplog_cf(&self.db)?;
        let prefix = oplog_tenant_repo_prefix(tenant_id, repo_id);
        let snapshot_key = vector_clock_snapshot_key(tenant_id, repo_id);
        let mut vector_clock = VectorClock::new();

        // The key alone carries everything this rebuild needs —
        // {tenant}\0{repo}\0{cluster_node_id}\0{op_seq: 8B BE}\0{ts: 8B BE} —
        // so scan KEYS ONLY. The previous implementation materialized every
        // operation (full node payloads included) into memory to read two
        // fields per op; on a mount-import-sized oplog that peaked around
        // 20 GB RSS and pinned a core for minutes at startup.
        //
        // Parse the fixed-width fields from the END of the key: the binary
        // seq/ts segments may themselves contain 0x00 bytes, so splitting on
        // the separator is wrong (same trap as ORDERED_CHILDREN keys).
        const TAIL: usize = 1 + 8 + 1 + 8; // \0 op_seq \0 timestamp

        let iter = self
            .db
            .iterator_cf(&cf, IteratorMode::From(&prefix, Direction::Forward));
        for item in iter {
            let (key, _value) =
                item.map_err(|e| Error::storage(format!("Iterator error: {}", e)))?;
            if !key.starts_with(&prefix[..prefix.len() - 1]) {
                break;
            }
            if key.as_ref() == snapshot_key.as_slice() {
                continue;
            }
            if key.len() < prefix.len() + TAIL {
                continue;
            }
            let node_end = key.len() - TAIL;
            let seq_start = node_end + 1;
            let Ok(op_seq) = decode_u64(&key[seq_start..seq_start + 8]) else {
                continue;
            };
            let cluster_node_id = String::from_utf8_lossy(&key[prefix.len()..node_end]);
            if op_seq > vector_clock.get(&cluster_node_id) {
                vector_clock.set(&cluster_node_id, op_seq);
            }
        }

        // Persist the rebuilt snapshot
        self.update_vector_clock_snapshot(tenant_id, repo_id, &vector_clock)?;

        Ok(vector_clock)
    }

    /// Verify vector clock snapshot consistency
    ///
    /// Compares the persisted snapshot with a freshly rebuilt one to detect
    /// any inconsistencies. This is useful for:
    /// - Periodic verification jobs
    /// - Debugging replication issues
    /// - Ensuring snapshot integrity
    ///
    /// # Returns
    ///
    /// - `Ok(true)` - Snapshot is consistent
    /// - `Ok(false)` - Snapshot differs from actual state (automatically corrected)
    /// - `Err(_)` - If verification fails
    pub fn verify_vector_clock_snapshot(&self, tenant_id: &str, repo_id: &str) -> Result<bool> {
        let snapshot = self.get_vector_clock_snapshot(tenant_id, repo_id)?;
        let rebuilt = self.rebuild_vector_clock_snapshot(tenant_id, repo_id)?;

        Ok(snapshot == rebuilt)
    }
}
