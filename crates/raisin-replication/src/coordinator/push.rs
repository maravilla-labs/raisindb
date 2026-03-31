//! Push-based replication and catch-up operations.
//!
//! Contains real-time push to peers, handling incoming pushes,
//! and cluster catch-up protocol execution.

use std::sync::Arc;

use tracing::{debug, info, warn};

use super::types::CoordinatorError;
use super::ReplicationCoordinator;
use crate::tcp_protocol::ReplicationMessage;
use crate::Operation;

impl ReplicationCoordinator {
    /// Push new operations to all connected peers (real-time replication)
    ///
    /// This is called after a transaction commits to immediately propagate changes.
    pub async fn push_to_all_peers(
        &self,
        operations: Vec<Operation>,
    ) -> Result<(), CoordinatorError> {
        debug!(
            count = operations.len(),
            realtime_push = self.sync_config.realtime_push,
            "push_to_all_peers called"
        );

        if !self.sync_config.realtime_push {
            debug!("Realtime push is disabled, operations will not be sent");
            return Ok(()); // Real-time push disabled
        }

        if operations.is_empty() {
            debug!("No operations to push");
            return Ok(());
        }

        self.metrics.operations_pushed.add(operations.len() as u64);

        info!(count = operations.len(), "Pushing operations to all peers");
        debug!("Getting peer statuses");

        let statuses = self.peer_manager.get_all_peer_status().await;
        debug!(peer_count = statuses.len(), "Found peers");

        let push_msg = ReplicationMessage::PushOperations { operations };

        for status in statuses {
            debug!(
                peer_id = %status.peer_id,
                state = ?status.state,
                "Peer state"
            );
            if !matches!(status.state, crate::ConnectionState::Connected) {
                debug!(peer_id = %status.peer_id, "Skipping peer (not connected)");
                continue;
            }

            let peer_id = status.peer_id.clone();
            let msg = push_msg.clone();
            let peer_manager = self.peer_manager.clone();

            debug!(peer_id = %peer_id, "Spawning task to push to peer");
            tokio::spawn(async move {
                debug!(peer_id = %peer_id, "Sending request to peer");
                match peer_manager.send_request(&peer_id, &msg).await {
                    Ok(ReplicationMessage::Ack { .. }) => {
                        info!(peer_id = %peer_id, "Push operations acknowledged");
                    }
                    Ok(other) => {
                        warn!(peer_id = %peer_id, "Unexpected push response: {:?}", other);
                    }
                    Err(e) => {
                        warn!(peer_id = %peer_id, error = %e, "Failed to push operations");
                    }
                }
            });
        }

        debug!("All push tasks spawned");
        Ok(())
    }

    /// Handle incoming push from a peer
    ///
    /// This is called when receiving a PushOperations message.
    pub async fn handle_push_from_peer(
        &self,
        peer_id: &str,
        operations: Vec<Operation>,
    ) -> Result<(), CoordinatorError> {
        info!(
            peer_id = %peer_id,
            count = operations.len(),
            "Received push from peer"
        );

        // Apply operations using CRDT replay engine
        let result = {
            let mut engine = self.replay_engine.write().await;
            engine.replay(operations)
        };

        if !result.applied.is_empty() {
            // Store applied operations
            self.storage
                .put_operations_batch(&result.applied)
                .await
                .map_err(|e| CoordinatorError::Storage(e.to_string()))?;

            // Send acknowledgment
            let ack_msg =
                ReplicationMessage::ack(result.applied.iter().map(|op| op.op_id).collect());
            self.peer_manager
                .send_message(peer_id, &ack_msg)
                .await
                .map_err(|e| CoordinatorError::Network(e.to_string()))?;

            info!(
                peer_id = %peer_id,
                applied = result.applied.len(),
                conflicts = result.conflicts.len(),
                "Applied pushed operations"
            );
        }

        Ok(())
    }

    /// Execute P2P cluster catch-up protocol
    ///
    /// This should be called when a fresh node joins the cluster to perform
    /// a full state synchronization. The catch-up coordinator is created externally
    /// with proper configuration.
    ///
    /// # Arguments
    /// * `catch_up_coordinator` - Configured catch-up coordinator instance
    ///
    /// # Returns
    /// Result of the catch-up operation
    ///
    /// # Example
    /// ```ignore
    /// let catch_up = CatchUpCoordinator::new(
    ///     node_id,
    ///     seed_peers,
    ///     data_dir,
    ///     staging_dir,
    /// );
    ///
    /// let result = coordinator.execute_cluster_catch_up(catch_up).await?;
    /// info!("Caught up from peer: {}", result.source_peer_id);
    /// ```
    pub async fn execute_cluster_catch_up(
        &self,
        catch_up_coordinator: crate::CatchUpCoordinator,
    ) -> Result<crate::CatchUpResult, CoordinatorError> {
        info!(
            cluster_node_id = %self.cluster_node_id,
            "Starting P2P cluster catch-up"
        );

        // Execute the catch-up protocol
        let result = catch_up_coordinator
            .execute_full_catch_up()
            .await
            .map_err(|e| CoordinatorError::CatchUp(format!("Catch-up failed: {}", e)))?;

        info!(
            source_peer = %result.source_peer_id,
            checkpoint_files = result.checkpoint_result.num_files,
            operations_applied = result.verification_result.operations_applied,
            "Cluster catch-up completed successfully"
        );

        Ok(result)
    }
}
