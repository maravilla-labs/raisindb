//! Synchronization logic for the replication coordinator.
//!
//! Contains pull-based sync, push-based replication, peer status queries,
//! and metrics collection methods.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use tracing::{debug, error, info, warn};

use super::types::{CoordinatorError, PeerClusterSnapshot, SyncStats};
use super::ReplicationCoordinator;
use crate::metrics::ReplicationMetrics;
use crate::tcp_protocol::ReplicationMessage;
use crate::Operation;

impl ReplicationCoordinator {
    /// Sync with a specific peer (pull missing operations)
    ///
    /// # Arguments
    /// * `peer_id` - ID of the peer to sync with
    /// * `tenant_repo_pairs` - Optional list of (tenant_id, repo_id) pairs to sync.
    ///   If None, syncs a default pair (useful for single-tenant deployments).
    pub async fn sync_with_peer_for_tenants(
        &self,
        peer_id: &str,
        tenant_repo_pairs: &[(String, String)],
    ) -> Result<(), CoordinatorError> {
        info!(
            peer_id = %peer_id,
            pairs = tenant_repo_pairs.len(),
            "Starting sync with peer"
        );

        if tenant_repo_pairs.is_empty() {
            debug!(peer_id = %peer_id, "No tenant/repository pairs provided");
            return Ok(());
        }

        // Sync each tenant/repo combination
        for (tenant_id, repo_id) in tenant_repo_pairs {
            self.sync_tenant_repo_with_peer(peer_id, tenant_id, repo_id)
                .await?;
        }

        Ok(())
    }

    /// Sync with a specific peer (pull missing operations) - simplified version
    ///
    /// This syncs a default tenant/repo pair for backward compatibility.
    /// For multi-tenant setups, use `sync_with_peer_for_tenants()` instead.
    pub async fn sync_with_peer(&self, peer_id: &str) -> Result<(), CoordinatorError> {
        info!(peer_id = %peer_id, "Starting sync with peer (default tenant/repo)");

        // Default to tenant1/repo1 for backward compatibility
        // TODO: Callers should use sync_with_peer_for_tenants() to specify exact pairs
        let tenant_id = "tenant1";
        let repo_id = "repo1";

        self.sync_tenant_repo_with_peer(peer_id, tenant_id, repo_id)
            .await
    }

    /// Sync a specific tenant/repo with a peer
    pub(crate) async fn sync_tenant_repo_with_peer(
        &self,
        peer_id: &str,
        tenant_id: &str,
        repo_id: &str,
    ) -> Result<(), CoordinatorError> {
        let start = Instant::now();
        self.metrics.sync_cycles.increment();

        // Get our current vector clock
        let local_vc = self
            .storage
            .get_vector_clock(tenant_id, repo_id)
            .await
            .map_err(|e| CoordinatorError::Storage(e.to_string()))?;

        // Request missing operations from peer
        let pull_request = ReplicationMessage::PullOperations {
            tenant_id: tenant_id.to_string(),
            repo_id: repo_id.to_string(),
            since_vector_clock: local_vc.clone(),
            branch_filter: None, // TODO: Apply branch filter from peer config
            limit: self.sync_config.batch_size,
        };

        let response = self
            .peer_manager
            .send_request(peer_id, &pull_request)
            .await
            .map_err(|e| CoordinatorError::Network(e.to_string()))?;

        // Receive operation batch
        match response {
            ReplicationMessage::OperationBatch {
                operations,
                has_more,
                ..
            } => {
                if !operations.is_empty() {
                    self.metrics
                        .operations_received
                        .add(operations.len() as u64);

                    debug!(
                        peer_id = %peer_id,
                        tenant_id = %tenant_id,
                        repo_id = %repo_id,
                        count = operations.len(),
                        "Received operations from peer"
                    );

                    // Deliver operations through causal buffer to ensure causal order
                    let mut deliverable_ops = Vec::new();
                    {
                        let mut buffer = self.causal_buffer.write().await;
                        for op in operations {
                            let mut delivered = buffer.deliver(op);
                            deliverable_ops.append(&mut delivered);
                        }
                    }

                    // Sort deliverable operations by priority (admin users first!)
                    crate::priority::sort_operations_by_priority(&mut deliverable_ops);

                    // Apply operations using CRDT replay engine
                    let result = {
                        let mut engine = self.replay_engine.write().await;
                        engine.replay(deliverable_ops)
                    };

                    if !result.applied.is_empty() {
                        // Track metrics
                        self.metrics
                            .operations_applied
                            .add(result.applied.len() as u64);
                        self.metrics
                            .operations_skipped
                            .add(result.skipped.len() as u64);
                        self.metrics
                            .conflicts_detected
                            .add(result.conflicts.len() as u64);

                        // Store applied operations
                        self.storage
                            .put_operations_batch(&result.applied)
                            .await
                            .map_err(|e| CoordinatorError::Storage(e.to_string()))?;

                        // Send acknowledgment
                        let ack_msg = ReplicationMessage::ack(
                            result.applied.iter().map(|op| op.op_id).collect(),
                        );
                        self.peer_manager
                            .send_message(peer_id, &ack_msg)
                            .await
                            .map_err(|e| CoordinatorError::Network(e.to_string()))?;

                        // Log buffer stats for monitoring
                        let buffer_stats = {
                            let buffer = self.causal_buffer.read().await;
                            buffer.stats().clone()
                        };
                        info!(
                            peer_id = %peer_id,
                            tenant_id = %tenant_id,
                            repo_id = %repo_id,
                            applied = result.applied.len(),
                            conflicts = result.conflicts.len(),
                            skipped = result.skipped.len(),
                            buffered = buffer_stats.current_buffered,
                            total_delivered = buffer_stats.total_delivered,
                            "Applied operations"
                        );
                    }

                    // If there are more operations, continue syncing
                    if has_more {
                        info!(
                            peer_id = %peer_id,
                            tenant_id = %tenant_id,
                            repo_id = %repo_id,
                            "More operations available, will sync in next cycle"
                        );
                        // The next periodic sync will pick up remaining operations
                    }
                } else {
                    debug!(
                        peer_id = %peer_id,
                        tenant_id = %tenant_id,
                        repo_id = %repo_id,
                        "No new operations from peer"
                    );
                }

                self.metrics.sync_duration.record(start.elapsed());
                Ok(())
            }
            ReplicationMessage::Error { message, .. } => {
                self.metrics.operations_failed.increment();
                self.metrics.sync_duration.record(start.elapsed());
                error!(
                    peer_id = %peer_id,
                    tenant_id = %tenant_id,
                    repo_id = %repo_id,
                    error = %message,
                    "Peer returned error"
                );
                Err(CoordinatorError::Protocol(message))
            }
            msg => {
                warn!(
                    peer_id = %peer_id,
                    tenant_id = %tenant_id,
                    repo_id = %repo_id,
                    "Unexpected message: {:?}",
                    msg
                );
                Err(CoordinatorError::Protocol(format!(
                    "Unexpected message: {:?}",
                    msg
                )))
            }
        }
    }
}
