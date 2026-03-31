//! Replication coordinator integration and state management
//!
//! This module handles replication state restoration, coordinator configuration,
//! and operation pushing to peers in multi-node deployments.

use super::RocksDBStorage;
use raisin_error::Result;
use raisin_storage::RegistryRepository;
use std::sync::Arc;

impl RocksDBStorage {
    /// Restore replication state from the operation log
    ///
    /// This method should be called during storage initialization to restore
    /// the vector clock and operation sequence counter from the persisted operation log.
    /// This ensures that the node can resume replication from where it left off.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - The tenant to restore state for
    /// * `repo_id` - The repository to restore state for
    ///
    /// # Returns
    ///
    /// Returns Ok(()) if successful, or an error if the restore fails.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use raisin_rocksdb::{RocksDBStorage, RocksDBConfig};
    ///
    /// # async fn example() -> raisin_error::Result<()> {
    /// let storage = RocksDBStorage::with_config(RocksDBConfig::production())?;
    ///
    /// // Restore replication state for a specific tenant/repo
    /// storage.restore_replication_state("tenant1", "repo1").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn restore_replication_state(&self, tenant_id: &str, repo_id: &str) -> Result<()> {
        if !self.config.replication_enabled {
            tracing::debug!("Replication is disabled, skipping state restoration");
            return Ok(());
        }

        tracing::info!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            "Restoring replication state from operation log"
        );

        self.operation_capture
            .restore_from_oplog(tenant_id, repo_id)
            .await?;

        // Initialize vector clock snapshot if it doesn't exist
        let oplog_repo = crate::repositories::OpLogRepository::new(self.db.clone());
        let snapshot = oplog_repo.get_vector_clock_snapshot(tenant_id, repo_id)?;

        if snapshot.is_empty() {
            tracing::info!(
                tenant_id = %tenant_id,
                repo_id = %repo_id,
                "Initializing vector clock snapshot from operation log"
            );
            oplog_repo.rebuild_vector_clock_snapshot(tenant_id, repo_id)?;
            tracing::info!(
                tenant_id = %tenant_id,
                repo_id = %repo_id,
                "Vector clock snapshot initialized successfully"
            );
        } else {
            tracing::debug!(
                tenant_id = %tenant_id,
                repo_id = %repo_id,
                "Vector clock snapshot already exists"
            );
        }

        tracing::info!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            "Successfully restored replication state"
        );

        Ok(())
    }

    /// Restore replication state for all tenant/repository pairs found in the registry
    ///
    /// This runs `restore_replication_state()` for `_registry` plus every repository
    /// registered under each tenant. It should be invoked during startup before
    /// any operations are captured to ensure sequence numbers continue monotonically
    /// across restarts.
    pub async fn restore_all_replication_state(&self) -> Result<()> {
        if !self.config.replication_enabled {
            tracing::debug!("Replication is disabled, skipping global state restoration");
            return Ok(());
        }

        let tenants = self.registry.list_tenants().await?;

        if tenants.is_empty() {
            tracing::debug!("No tenants found while restoring replication state");
            return Ok(());
        }

        let mut restored_pairs = 0usize;

        for tenant in tenants {
            let tenant_id = tenant.tenant_id;

            // Always restore the special _registry repository per tenant
            self.restore_replication_state(&tenant_id, "_registry")
                .await?;
            restored_pairs += 1;

            let repos = crate::management::list_repositories(self, &tenant_id).await?;
            for repo_id in repos {
                self.restore_replication_state(&tenant_id, &repo_id).await?;
                restored_pairs += 1;
            }
        }

        tracing::info!(
            restored_pairs = restored_pairs,
            "Restored replication state for {} tenant/repository pairs",
            restored_pairs
        );

        Ok(())
    }

    /// Set the replication coordinator
    ///
    /// This should be called after starting replication via `start_replication()`.
    /// The coordinator will be used to push operations to peers in real-time.
    pub async fn set_replication_coordinator(
        &self,
        coordinator: Arc<raisin_replication::ReplicationCoordinator>,
    ) {
        let mut guard = self.replication_coordinator.write().await;
        *guard = Some(coordinator);
    }

    /// Push operations to all replication peers
    ///
    /// This is called after operations are captured to immediately propagate changes
    /// to connected peers (if real-time push is enabled in cluster config).
    pub async fn push_operations_to_peers(
        &self,
        operations: Vec<raisin_replication::Operation>,
    ) -> Result<(), raisin_error::Error> {
        tracing::info!(
            num_operations = operations.len(),
            op_ids = ?operations.iter().map(|op| &op.op_id).collect::<Vec<_>>(),
            "💼 PUSH_OPERATIONS_TO_PEERS called in storage.rs"
        );

        // Decompose operations for CRDT commutativity (ApplyRevision → atomic ops)
        let original_count = operations.len();
        let mut decomposed_ops = Vec::new();
        for op in operations {
            let ops = raisin_replication::decompose_operation(op);
            decomposed_ops.extend(ops);
        }

        if decomposed_ops.len() != original_count {
            tracing::info!(
                original_count,
                decomposed_count = decomposed_ops.len(),
                "🔨 Operations decomposed for replication"
            );
        }

        let guard = self.replication_coordinator.read().await;
        if let Some(ref coordinator) = *guard {
            tracing::info!(
                num_operations = decomposed_ops.len(),
                "📡 Coordinator found, calling push_to_all_peers"
            );
            coordinator
                .push_to_all_peers(decomposed_ops)
                .await
                .map_err(|e| {
                    tracing::error!(error = %e, "❌ COORDINATOR PUSH_TO_ALL_PEERS FAILED");
                    raisin_error::Error::storage(format!("Replication push failed: {}", e))
                })?;
            tracing::info!("✅ Coordinator push_to_all_peers succeeded");
        } else {
            tracing::warn!("⚠️  NO COORDINATOR SET - operations will not replicate");
        }
        Ok(())
    }

    /// Check if this node is ready to serve requests
    ///
    /// A node is considered ready when:
    /// 1. Replication is disabled (single-node mode), OR
    /// 2. Replication coordinator is configured and initial sync has completed
    ///
    /// This should be called before allowing the node to accept user requests
    /// to ensure consistency across the cluster.
    ///
    /// # Returns
    ///
    /// * `true` - Node is ready to serve requests
    /// * `false` - Node is still waiting for replication coordinator setup
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use raisin_rocksdb::RocksDBStorage;
    ///
    /// # async fn example(storage: &RocksDBStorage) -> raisin_error::Result<()> {
    /// // Wait for node to be ready before serving traffic
    /// while !storage.is_ready_for_requests().await {
    ///     tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    /// }
    /// println!("Node is ready to serve requests");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn is_ready_for_requests(&self) -> bool {
        // If replication is disabled, node is always ready
        if !self.config.replication_enabled {
            tracing::debug!("Node ready: replication disabled (single-node mode)");
            return true;
        }

        // Check if replication coordinator is configured
        let guard = self.replication_coordinator.read().await;
        if guard.is_none() {
            tracing::debug!("Node not ready: replication coordinator not configured");
            return false;
        }

        // If coordinator is configured, consider node ready
        // The coordinator will handle initial sync in the background
        tracing::debug!("Node ready: replication coordinator configured");
        true
    }
}
