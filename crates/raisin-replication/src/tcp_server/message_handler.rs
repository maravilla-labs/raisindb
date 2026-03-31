//! Message handler for incoming replication protocol messages
//!
//! Handles all replication protocol messages including data sync,
//! catch-up protocol, and index transfer requests.

use tokio::net::TcpStream;
use tracing::{debug, info, warn};

use crate::coordinator::CoordinatorError;
use crate::tcp_protocol::ReplicationMessage;
use crate::VectorClock;

use super::ReplicationServer;

impl ReplicationServer {
    /// Handle a replication protocol message
    pub(super) async fn handle_message(
        &self,
        stream: &mut TcpStream,
        peer_id: &str,
        message: ReplicationMessage,
    ) -> Result<(), CoordinatorError> {
        match message {
            ReplicationMessage::GetVectorClock { tenant_id, repo_id } => {
                self.handle_get_vector_clock(stream, &tenant_id, &repo_id)
                    .await
            }
            ReplicationMessage::PullOperations {
                tenant_id,
                repo_id,
                since_vector_clock,
                limit,
                ..
            } => {
                self.handle_pull_operations(
                    stream,
                    &tenant_id,
                    &repo_id,
                    &since_vector_clock,
                    limit,
                )
                .await
            }
            ReplicationMessage::PushOperations { operations } => {
                self.handle_push_operations(stream, peer_id, operations)
                    .await
            }
            ReplicationMessage::Ping { timestamp_ms } => {
                let pong = ReplicationMessage::Pong { timestamp_ms };
                self.send_message(stream, &pong).await
            }
            ReplicationMessage::Ack { op_ids } => {
                debug!(peer_id = %peer_id, count = op_ids.len(), "Received acknowledgment from peer");
                Ok(())
            }
            ReplicationMessage::VectorClockResponse { .. }
            | ReplicationMessage::OperationBatch { .. }
            | ReplicationMessage::Pong { .. }
            | ReplicationMessage::HelloAck { .. }
            | ReplicationMessage::Hello { .. } => {
                debug!(peer_id = %peer_id, "Received response message type");
                Ok(())
            }
            // Catch-up protocol messages
            ReplicationMessage::GetClusterStatus => {
                self.handle_get_cluster_status(stream, peer_id).await
            }
            ReplicationMessage::InitiateCatchUp {
                requesting_node,
                local_vector_clock,
            } => {
                self.handle_initiate_catch_up(
                    stream,
                    peer_id,
                    &requesting_node,
                    &local_vector_clock,
                )
                .await
            }
            ReplicationMessage::RequestCheckpoint {
                snapshot_id,
                max_parallel_files,
            } => {
                self.handle_request_checkpoint(stream, peer_id, &snapshot_id, max_parallel_files)
                    .await
            }
            ReplicationMessage::RequestLogTail {
                since_vector_clock,
                max_operations,
            } => {
                self.handle_request_log_tail(stream, peer_id, &since_vector_clock, max_operations)
                    .await
            }
            ReplicationMessage::NodeReady {
                node_id,
                vector_clock: _,
            } => {
                info!(
                    peer_id = %peer_id,
                    ready_node = %node_id,
                    "Peer announcing node ready after catch-up"
                );
                Ok(())
            }
            // Index transfer messages
            ReplicationMessage::RequestTantivyIndexList => {
                self.handle_request_tantivy_index_list(stream, peer_id)
                    .await
            }
            ReplicationMessage::RequestHnswIndexList => {
                self.handle_request_hnsw_index_list(stream, peer_id).await
            }
            ReplicationMessage::RequestTantivyIndex {
                tenant_id,
                repo_id,
                branch,
            } => {
                self.handle_request_tantivy_index(stream, peer_id, &tenant_id, &repo_id, &branch)
                    .await
            }
            ReplicationMessage::RequestHnswIndex {
                tenant_id,
                repo_id,
                branch,
            } => {
                self.handle_request_hnsw_index(stream, peer_id, &tenant_id, &repo_id, &branch)
                    .await
            }
            _ => {
                warn!(peer_id = %peer_id, "Unexpected message type: {:?}", message);
                let error_msg = ReplicationMessage::error(
                    crate::tcp_protocol::ErrorCode::ProtocolError,
                    format!("Unexpected message type: {:?}", message),
                );
                self.send_message(stream, &error_msg).await
            }
        }
    }

    async fn handle_get_vector_clock(
        &self,
        stream: &mut TcpStream,
        tenant_id: &str,
        repo_id: &str,
    ) -> Result<(), CoordinatorError> {
        let vector_clock = self
            .storage
            .get_vector_clock(tenant_id, repo_id)
            .await
            .map_err(|e| CoordinatorError::Storage(e.to_string()))?;

        let response = ReplicationMessage::VectorClockResponse {
            tenant_id: tenant_id.to_string(),
            repo_id: repo_id.to_string(),
            vector_clock,
        };
        self.send_message(stream, &response).await
    }

    async fn handle_pull_operations(
        &self,
        stream: &mut TcpStream,
        tenant_id: &str,
        repo_id: &str,
        since_vector_clock: &VectorClock,
        limit: usize,
    ) -> Result<(), CoordinatorError> {
        let operations = self
            .storage
            .get_operations_since(tenant_id, repo_id, since_vector_clock, limit)
            .await
            .map_err(|e| CoordinatorError::Storage(e.to_string()))?;

        let has_more = operations.len() >= limit;
        let total_available = operations.len();

        let response = ReplicationMessage::OperationBatch {
            operations,
            has_more,
            total_available,
        };

        self.send_message(stream, &response).await
    }

    async fn handle_push_operations(
        &self,
        stream: &mut TcpStream,
        peer_id: &str,
        operations: Vec<crate::Operation>,
    ) -> Result<(), CoordinatorError> {
        info!(
            peer_id = %peer_id,
            count = operations.len(),
            "Received push operations from peer"
        );

        self.storage
            .put_operations_batch(&operations)
            .await
            .map_err(|e| CoordinatorError::Storage(e.to_string()))?;

        let op_ids: Vec<_> = operations.iter().map(|op| op.op_id).collect();
        let ack = ReplicationMessage::ack(op_ids);
        self.send_message(stream, &ack).await
    }

    async fn handle_get_cluster_status(
        &self,
        stream: &mut TcpStream,
        peer_id: &str,
    ) -> Result<(), CoordinatorError> {
        debug!(peer_id = %peer_id, "Peer requesting cluster status");

        let stats = match self.storage.get_cluster_stats().await {
            Ok(stats) => stats,
            Err(e) => {
                warn!(error = %e, "Failed to get cluster stats, returning empty");
                crate::ClusterStorageStats {
                    max_vector_clock: VectorClock::new(),
                    num_tenants: 0,
                    num_repos: 0,
                    tenant_repos: Vec::new(),
                }
            }
        };

        let log_index = stats.max_vector_clock.as_map().values().sum::<u64>();

        let response = ReplicationMessage::ClusterStatusResponse {
            node_id: self.cluster_node_id.clone(),
            log_index,
            max_vector_clock: stats.max_vector_clock.clone(),
            num_tenants: stats.num_tenants,
            num_repos: stats.num_repos,
            last_update_timestamp_ms: 0,
            known_peers: Vec::new(),
            tenant_repos: stats.tenant_repos.clone(),
            storage_size_bytes: None,
        };

        debug!(
            num_tenants = stats.num_tenants,
            num_repos = stats.num_repos,
            log_index = log_index,
            vc_size = stats.max_vector_clock.as_map().len(),
            "Sending cluster status response"
        );

        self.send_message(stream, &response).await
    }

    async fn handle_initiate_catch_up(
        &self,
        stream: &mut TcpStream,
        peer_id: &str,
        requesting_node: &str,
        _local_vector_clock: &VectorClock,
    ) -> Result<(), CoordinatorError> {
        info!(
            peer_id = %peer_id,
            requesting_node = %requesting_node,
            "Peer initiating catch-up"
        );

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let snapshot_id = format!("snapshot_{}_{}", requesting_node, timestamp);
        let snapshot_vector_clock = VectorClock::new();

        let response = ReplicationMessage::CatchUpAck {
            source_node: self.cluster_node_id.clone(),
            snapshot_id,
            snapshot_vector_clock,
            estimated_transfer_size_bytes: None,
        };

        self.send_message(stream, &response).await
    }

    async fn handle_request_checkpoint(
        &self,
        stream: &mut TcpStream,
        peer_id: &str,
        snapshot_id: &str,
        max_parallel_files: u8,
    ) -> Result<(), CoordinatorError> {
        info!(
            peer_id = %peer_id,
            snapshot_id = %snapshot_id,
            max_parallel = max_parallel_files,
            "Peer requesting checkpoint"
        );

        if let Some(ref checkpoint_provider) = self.checkpoint_provider {
            checkpoint_provider
                .handle_checkpoint_request(stream, snapshot_id, max_parallel_files)
                .await?;

            info!(
                peer_id = %peer_id,
                snapshot_id = %snapshot_id,
                "Checkpoint served successfully"
            );
            Ok(())
        } else {
            warn!(
                peer_id = %peer_id,
                "CheckpointProvider not configured on this node"
            );
            let error_msg = ReplicationMessage::error(
                crate::tcp_protocol::ErrorCode::InternalError,
                "Checkpoint serving not available on this node".to_string(),
            );
            self.send_message(stream, &error_msg).await
        }
    }

    async fn handle_request_log_tail(
        &self,
        stream: &mut TcpStream,
        peer_id: &str,
        since_vector_clock: &VectorClock,
        max_operations: u32,
    ) -> Result<(), CoordinatorError> {
        info!(
            peer_id = %peer_id,
            max_ops = max_operations,
            "Peer requesting log tail for catch-up"
        );

        let max_ops = max_operations as usize;
        let mut operations = Vec::new();
        let mut has_more = false;

        let stats = self
            .storage
            .get_cluster_stats()
            .await
            .map_err(|e| CoordinatorError::Storage(e.to_string()))?;

        let mut remaining = if max_ops == 0 {
            info!(peer_id = %peer_id, "RequestLogTail max_operations=0, returning empty batch");
            0usize
        } else {
            max_ops
        };

        for (tenant_id, repo_id) in &stats.tenant_repos {
            if remaining == 0 {
                has_more = true;
                break;
            }

            let repo_limit = remaining;
            let repo_ops = self
                .storage
                .get_operations_since(tenant_id, repo_id, since_vector_clock, repo_limit)
                .await
                .map_err(|e| CoordinatorError::Storage(e.to_string()))?;

            if repo_limit > 0 && repo_ops.len() == repo_limit {
                has_more = true;
            }

            remaining = remaining.saturating_sub(repo_ops.len());
            operations.extend(repo_ops);
        }

        if remaining == 0 && max_ops > 0 {
            has_more = true;
        }

        let returned = operations.len();
        let response = ReplicationMessage::LogTailResponse {
            operations,
            peer_vector_clock: stats.max_vector_clock.clone(),
            has_more,
        };

        info!(
            peer_id = %peer_id,
            returned_operations = returned,
            tenant_repo_pairs = stats.tenant_repos.len(),
            has_more = has_more,
            "Served log tail request"
        );

        self.send_message(stream, &response).await
    }

    async fn handle_request_tantivy_index_list(
        &self,
        stream: &mut TcpStream,
        peer_id: &str,
    ) -> Result<(), CoordinatorError> {
        info!(peer_id = %peer_id, "Peer requesting Tantivy index list");

        if let Some(ref checkpoint_provider) = self.checkpoint_provider {
            checkpoint_provider
                .handle_tantivy_index_list_request(stream)
                .await?;
            info!(peer_id = %peer_id, "Tantivy index list sent successfully");
            Ok(())
        } else {
            warn!(peer_id = %peer_id, "CheckpointProvider not configured on this node");
            let error_msg = ReplicationMessage::error(
                crate::tcp_protocol::ErrorCode::InternalError,
                "Tantivy index serving not available on this node".to_string(),
            );
            self.send_message(stream, &error_msg).await
        }
    }

    async fn handle_request_hnsw_index_list(
        &self,
        stream: &mut TcpStream,
        peer_id: &str,
    ) -> Result<(), CoordinatorError> {
        info!(peer_id = %peer_id, "Peer requesting HNSW index list");

        if let Some(ref checkpoint_provider) = self.checkpoint_provider {
            checkpoint_provider
                .handle_hnsw_index_list_request(stream)
                .await?;
            info!(peer_id = %peer_id, "HNSW index list sent successfully");
            Ok(())
        } else {
            warn!(peer_id = %peer_id, "CheckpointProvider not configured on this node");
            let error_msg = ReplicationMessage::error(
                crate::tcp_protocol::ErrorCode::InternalError,
                "HNSW index serving not available on this node".to_string(),
            );
            self.send_message(stream, &error_msg).await
        }
    }

    async fn handle_request_tantivy_index(
        &self,
        stream: &mut TcpStream,
        peer_id: &str,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<(), CoordinatorError> {
        info!(
            peer_id = %peer_id,
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            branch = %branch,
            "Peer requesting Tantivy index"
        );

        if let Some(ref checkpoint_provider) = self.checkpoint_provider {
            checkpoint_provider
                .handle_tantivy_index_request(stream, tenant_id, repo_id, branch)
                .await?;
            info!(
                peer_id = %peer_id,
                tenant_id = %tenant_id,
                repo_id = %repo_id,
                branch = %branch,
                "Tantivy index served successfully"
            );
            Ok(())
        } else {
            warn!(peer_id = %peer_id, "CheckpointProvider not configured on this node");
            let error_msg = ReplicationMessage::error(
                crate::tcp_protocol::ErrorCode::InternalError,
                "Tantivy index serving not available on this node".to_string(),
            );
            self.send_message(stream, &error_msg).await
        }
    }

    async fn handle_request_hnsw_index(
        &self,
        stream: &mut TcpStream,
        peer_id: &str,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<(), CoordinatorError> {
        info!(
            peer_id = %peer_id,
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            branch = %branch,
            "Peer requesting HNSW index"
        );

        if let Some(ref checkpoint_provider) = self.checkpoint_provider {
            checkpoint_provider
                .handle_hnsw_index_request(stream, tenant_id, repo_id, branch)
                .await?;
            info!(
                peer_id = %peer_id,
                tenant_id = %tenant_id,
                repo_id = %repo_id,
                branch = %branch,
                "HNSW index served successfully"
            );
            Ok(())
        } else {
            warn!(peer_id = %peer_id, "CheckpointProvider not configured on this node");
            let error_msg = ReplicationMessage::error(
                crate::tcp_protocol::ErrorCode::InternalError,
                "HNSW index serving not available on this node".to_string(),
            );
            self.send_message(stream, &error_msg).await
        }
    }
}
