//! Log verification and replay (Phase 7).
//!
//! Queries all peers for operations beyond the consensus log index,
//! resolves conflicts using CRDT conflict resolution, and applies
//! the resolved operations to local storage.

use super::types::{ConsensusState, PeerStatus, VerificationResult};
use super::CatchUpCoordinator;
use crate::{ReplicationMessage, VectorClock};
use raisin_error::{Error, Result};
use std::collections::HashMap;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tracing::{error, info, warn};

impl CatchUpCoordinator {
    /// Phase 7: Verify and apply log tail from all peers
    pub(super) async fn verify_and_apply_log_tail(
        &self,
        peer_statuses: &[PeerStatus],
        consensus: &ConsensusState,
    ) -> Result<VerificationResult> {
        info!(
            consensus_log_index = consensus.log_index,
            num_peers = peer_statuses.len(),
            "Phase 7: Verifying and applying log tail from all peers"
        );

        let local_vector_clock = self.local_vector_clock().await;

        info!(
            local_clock_entries = local_vector_clock.as_map().len(),
            "Local vector clock prepared for log tail requests"
        );

        // Query all peers for operations beyond consensus log index
        let mut peer_logs = HashMap::new();
        let mut successful_peers = 0usize;

        for peer in peer_statuses {
            match self.query_log_tail(peer, &local_vector_clock).await {
                Ok(ops) => {
                    info!(
                        peer_id = %peer.node_id,
                        num_operations = ops.len(),
                        "Received log tail from peer"
                    );
                    peer_logs.insert(peer.node_id.clone(), ops);
                    successful_peers += 1;
                }
                Err(e) => {
                    warn!(
                        peer_id = %peer.node_id,
                        error = %e,
                        "Failed to query log tail from peer"
                    );
                }
            }
        }

        if successful_peers == 0 {
            return Err(Error::Backend(
                "Failed to retrieve log tail from any peers - checkpoint fallback required"
                    .to_string(),
            ));
        }

        // Resolve divergent logs using conflict resolution
        let resolved_ops = self
            .conflict_resolver
            .resolve_divergent_logs(Vec::new(), peer_logs)
            .map_err(|e| Error::Backend(format!("Conflict resolution failed: {}", e)))?;

        info!(
            operations_to_apply = resolved_ops.len(),
            "Operations resolved and ready for replay"
        );

        // Detect conflicts
        let conflicts = self.conflict_resolver.detect_conflicts(&resolved_ops);

        let conflicts_resolved = conflicts.len();

        // Apply operations to local storage
        let operations_applied = if let Some(storage) = &self.storage {
            if !resolved_ops.is_empty() {
                info!(
                    num_operations = resolved_ops.len(),
                    "Applying resolved operations to local storage"
                );

                // Log critical operation types for diagnostics
                let update_repo_count = resolved_ops
                    .iter()
                    .filter(|op| matches!(op.op_type, crate::OpType::UpdateRepository { .. }))
                    .count();
                let update_nodetype_count = resolved_ops
                    .iter()
                    .filter(|op| matches!(op.op_type, crate::OpType::UpdateNodeType { .. }))
                    .count();

                if update_repo_count > 0 || update_nodetype_count > 0 {
                    info!(
                        update_repo_ops = update_repo_count,
                        update_nodetype_ops = update_nodetype_count,
                        "Log tail contains critical workspace initialization operations"
                    );
                }

                // Apply all operations as a batch
                match storage.put_operations_batch(&resolved_ops).await {
                    Ok(_) => {
                        info!(
                            operations_applied = resolved_ops.len(),
                            "Operations applied to local storage"
                        );

                        // Verify that critical operations were applied successfully
                        if update_repo_count > 0 {
                            info!(
                                update_repo_count = update_repo_count,
                                "Applied UpdateRepository operations - event handlers should initialize workspaces"
                            );
                        }

                        if update_nodetype_count > 0 {
                            info!(
                                update_nodetype_count = update_nodetype_count,
                                "Applied UpdateNodeType operations during log tail replay"
                            );
                        }

                        resolved_ops.len()
                    }
                    Err(e) => {
                        error!(
                            error = %e,
                            num_operations = resolved_ops.len(),
                            "Failed to apply operations batch during log tail replay - this is critical!"
                        );

                        // Log which operations failed
                        for (idx, op) in resolved_ops.iter().enumerate() {
                            if let crate::OpType::UpdateRepository { .. }
                            | crate::OpType::UpdateNodeType { .. } = op.op_type
                            {
                                warn!(
                                    op_index = idx,
                                    op_id = %op.op_id,
                                    op_type = ?op.op_type,
                                    tenant_id = %op.tenant_id,
                                    repo_id = %op.repo_id,
                                    "Critical operation may have failed to apply"
                                );
                            }
                        }

                        0
                    }
                }
            } else {
                info!("No operations to apply in log tail - checkpoint is up to date");
                0
            }
        } else {
            warn!("No storage backend configured, operations not applied");
            0
        };

        Ok(VerificationResult {
            operations_applied,
            conflicts_resolved,
        })
    }

    /// Query log tail from a single peer
    async fn query_log_tail(
        &self,
        peer: &PeerStatus,
        since_vector_clock: &VectorClock,
    ) -> Result<Vec<crate::Operation>> {
        // Connect to peer
        let mut stream = timeout(self.network_timeout, TcpStream::connect(&peer.address))
            .await
            .map_err(|_| Error::Backend("Connection timeout".to_string()))?
            .map_err(|e| Error::Backend(format!("Failed to connect to peer: {}", e)))?;

        // Send Hello handshake first (required by ReplicationServer)
        let hello = ReplicationMessage::Hello {
            cluster_node_id: self.local_node_id.clone(),
            protocol_version: crate::tcp_protocol::PROTOCOL_VERSION,
            metadata: None,
        };
        Self::send_message(&mut stream, &hello).await?;

        // Wait for HelloAck
        let hello_response = Self::receive_message(&mut stream).await?;
        match hello_response {
            ReplicationMessage::HelloAck { .. } => {
                // Handshake successful, continue
            }
            _ => {
                return Err(Error::Backend(
                    "Expected HelloAck response to Hello".to_string(),
                ));
            }
        }

        // Send RequestLogTail
        let request = ReplicationMessage::RequestLogTail {
            since_vector_clock: since_vector_clock.clone(),
            max_operations: 10000, // Limit to prevent OOM
        };

        Self::send_message(&mut stream, &request).await?;

        // Receive LogTailResponse
        let response = Self::receive_message(&mut stream).await?;

        match response {
            ReplicationMessage::LogTailResponse {
                operations,
                peer_vector_clock: _,
                has_more,
            } => {
                if has_more {
                    warn!(
                        peer_id = %peer.node_id,
                        "Peer has more operations beyond limit"
                    );
                }
                Ok(operations)
            }
            _ => Err(Error::Backend(
                "Unexpected response to RequestLogTail".to_string(),
            )),
        }
    }
}
