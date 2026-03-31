//! RocksDB-backed operation log storage for replication.
//!
//! Implements OperationLogStorage from raisin-replication using RocksDB
//! as the persistent backend.

use std::sync::Arc;

use async_trait::async_trait;
use raisin_replication::{Operation, OperationLogStorage, StorageError, VectorClock};
use raisin_storage::Storage;

use crate::repositories::OpLogRepository;
use crate::RocksDBStorage;

use super::enumerate_all_tenant_repos;

/// Implements OperationLogStorage trait for RocksDB
pub struct RocksDbOperationLogStorage {
    oplog_repo: OpLogRepository,
    db: Arc<RocksDBStorage>,
}

impl RocksDbOperationLogStorage {
    /// Create a new RocksDB operation log storage
    pub fn new(db: Arc<RocksDBStorage>) -> Self {
        Self {
            oplog_repo: OpLogRepository::new(db.db().clone()),
            db: db.clone(),
        }
    }
}

#[async_trait]
impl OperationLogStorage for RocksDbOperationLogStorage {
    async fn get_operations_since(
        &self,
        tenant_id: &str,
        repo_id: &str,
        since_vc: &VectorClock,
        limit: usize,
    ) -> Result<Vec<Operation>, StorageError> {
        let ops = self
            .oplog_repo
            .get_missing_operations(tenant_id, repo_id, since_vc, Some(limit))
            .map_err(|e| StorageError::Backend(e.to_string()))?;

        Ok(ops)
    }

    async fn put_operations_batch(&self, ops: &[Operation]) -> Result<(), StorageError> {
        if ops.is_empty() {
            return Ok(());
        }

        // Step 1: Write operations to the operation log
        // This ensures they're persisted even if application fails
        self.oplog_repo
            .put_operations_batch(ops)
            .map_err(|e| StorageError::Backend(e.to_string()))?;

        // Step 2: Apply operations to the actual database
        // This is where the magic happens - operations become actual data changes
        let branch_repo = Arc::new(crate::repositories::BranchRepositoryImpl::new(
            self.db.db().clone(),
        ));
        let applicator = crate::replication::OperationApplicator::new(
            self.db.db().clone(),
            self.db.event_bus().clone(),
            branch_repo,
        );

        // Sort operations by op_seq to ensure correct application order
        // This is CRITICAL for UpdateBranch operations which must come AFTER
        // the operations that create the revisions they reference
        let mut sorted_ops: Vec<&Operation> = ops.iter().collect();
        sorted_ops.sort_by_key(|op| op.op_seq);

        tracing::info!(
            "📋 Applying {} operations in op_seq order (seq {} to {})",
            sorted_ops.len(),
            sorted_ops.first().map(|o| o.op_seq).unwrap_or(0),
            sorted_ops.last().map(|o| o.op_seq).unwrap_or(0)
        );

        for op in sorted_ops {
            // Apply each operation
            // If application fails, we log but don't fail the batch
            // The operation is already in the log, so it can be retried
            if let Err(e) = applicator.apply_operation(op).await {
                tracing::error!(
                    "Failed to apply operation {} (seq={}) from node {}: {}",
                    op.op_id,
                    op.op_seq,
                    op.cluster_node_id,
                    e
                );
                // Continue with next operation instead of failing entire batch
            }
        }

        // CRITICAL FIX: Update vector clock snapshot after applying operations
        // This ensures the next sync cycle knows what operations we've received
        // Without this, nodes request the same operations repeatedly (infinite loop)
        for op in ops {
            if let Err(e) = self.oplog_repo.increment_vector_clock_for_node(
                &op.tenant_id,
                &op.repo_id,
                &op.cluster_node_id,
                op.op_seq,
            ) {
                tracing::error!(
                    "Failed to update vector clock for operation {} (seq={}) from node {}: {}",
                    op.op_id,
                    op.op_seq,
                    op.cluster_node_id,
                    e
                );
                // Don't fail the batch, but log the error
                // The operation is applied, but vector clock might be stale
            }
        }

        tracing::debug!(
            "Updated vector clock snapshot for {} operations across {} tenant/repo pairs",
            ops.len(),
            ops.iter()
                .map(|o| format!("{}/{}", o.tenant_id, o.repo_id))
                .collect::<std::collections::HashSet<_>>()
                .len()
        );

        // Step 3: Emit event for large batches (potential catch-up scenario)
        // This allows the job system to trigger lazy indexing
        const BATCH_THRESHOLD: usize = 10; // Consider batches of 10+ operations as catch-up

        if ops.len() >= BATCH_THRESHOLD {
            // Extract tenant_id and repo_id from first operation
            // All operations in a batch should have the same tenant/repo
            let tenant_id = &ops[0].tenant_id;
            let repo_id = &ops[0].repo_id;

            tracing::info!(
                "Emitting OperationBatchApplied event: tenant={}, repo={}, count={}",
                tenant_id,
                repo_id,
                ops.len()
            );

            let event = raisin_events::Event::Replication(raisin_events::ReplicationEvent {
                tenant_id: tenant_id.clone(),
                repository_id: repo_id.clone(),
                branch: None,    // Batch may contain multiple branches
                workspace: None, // Batch may contain multiple workspaces
                operation_count: ops.len(),
                kind: raisin_events::ReplicationEventKind::OperationBatchApplied,
                metadata: None,
            });

            self.db.event_bus().publish(event);
        }

        Ok(())
    }

    async fn get_vector_clock(
        &self,
        tenant_id: &str,
        repo_id: &str,
    ) -> Result<VectorClock, StorageError> {
        self.oplog_repo
            .get_vector_clock_snapshot(tenant_id, repo_id)
            .map_err(|e| StorageError::Backend(e.to_string()))
    }

    async fn get_operations_for_node(
        &self,
        tenant_id: &str,
        repo_id: &str,
        node_id: &str,
        since_seq: u64,
        limit: usize,
    ) -> Result<Vec<Operation>, StorageError> {
        let mut ops = self
            .oplog_repo
            .get_operations_from_seq(tenant_id, repo_id, node_id, since_seq)
            .map_err(|e| StorageError::Backend(e.to_string()))?;

        // Apply limit
        ops.truncate(limit);
        Ok(ops)
    }

    async fn get_cluster_stats(
        &self,
    ) -> Result<raisin_replication::ClusterStorageStats, StorageError> {
        // Enumerate all tenant/repo pairs from database
        let tenant_repos = enumerate_all_tenant_repos(&self.db)
            .await
            .map_err(|e| StorageError::Backend(e.to_string()))?;

        // Aggregate vector clocks across all tenant/repo pairs
        let mut max_vector_clock = VectorClock::new();

        for (tenant_id, repo_id) in &tenant_repos {
            match self
                .oplog_repo
                .get_vector_clock_snapshot(tenant_id, repo_id)
            {
                Ok(vc) => {
                    // CRITICAL: If snapshot is empty but operations exist, rebuild it
                    // This handles cases where the snapshot was never initialized
                    if vc.is_empty() {
                        tracing::debug!(
                            tenant_id = %tenant_id,
                            repo_id = %repo_id,
                            "Vector clock snapshot is empty, checking if rebuild needed"
                        );

                        // Try to rebuild from operation log
                        match self
                            .oplog_repo
                            .rebuild_vector_clock_snapshot(tenant_id, repo_id)
                        {
                            Ok(rebuilt_vc) => {
                                if !rebuilt_vc.is_empty() {
                                    tracing::info!(
                                        tenant_id = %tenant_id,
                                        repo_id = %repo_id,
                                        vc_entries = rebuilt_vc.len(),
                                        "Rebuilt vector clock snapshot from operation log"
                                    );
                                    max_vector_clock.merge(&rebuilt_vc);
                                } else {
                                    tracing::debug!(
                                        tenant_id = %tenant_id,
                                        repo_id = %repo_id,
                                        "No operations found for this tenant/repo"
                                    );
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    tenant_id = %tenant_id,
                                    repo_id = %repo_id,
                                    error = %e,
                                    "Failed to rebuild vector clock snapshot"
                                );
                            }
                        }
                    } else {
                        // Snapshot exists and is non-empty, use it
                        max_vector_clock.merge(&vc);
                    }
                }
                Err(e) => {
                    tracing::debug!(
                        tenant_id = %tenant_id,
                        repo_id = %repo_id,
                        error = %e,
                        "Failed to get vector clock during stats aggregation"
                    );
                }
            }
        }

        // Count unique tenants
        let mut unique_tenants = std::collections::HashSet::new();
        for (tenant_id, _) in &tenant_repos {
            unique_tenants.insert(tenant_id.clone());
        }

        Ok(raisin_replication::ClusterStorageStats {
            max_vector_clock,
            num_tenants: unique_tenants.len(),
            num_repos: tenant_repos.len(),
            tenant_repos,
        })
    }
}
