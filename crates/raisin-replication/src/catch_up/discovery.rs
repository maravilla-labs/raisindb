//! Cluster discovery and consensus determination (Phases 1-3).
//!
//! Implements peer discovery, consensus log index calculation,
//! and source peer selection for the catch-up protocol.

use super::types::{ConsensusState, PeerStatus};
use super::CatchUpCoordinator;
use crate::{ReplicationMessage, VectorClock};
use raisin_error::{Error, Result};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tracing::{info, warn};

impl CatchUpCoordinator {
    /// Phase 1: Discover cluster by querying seed peers
    pub(super) async fn discover_cluster(&self) -> Result<Vec<PeerStatus>> {
        info!("Phase 1: Discovering cluster topology");

        let mut peer_statuses = Vec::new();
        let mut connection_futures = Vec::new();

        // Connect to all seed peers concurrently
        for peer_addr in &self.seed_peers {
            let peer_addr = peer_addr.clone();
            let node_id = self.local_node_id.clone();

            let timeout_duration = self.network_timeout;
            connection_futures.push(async move {
                match Self::query_peer_status(&peer_addr, &node_id).await {
                    Ok(status) => Some(status),
                    Err(e) => {
                        warn!(
                            peer_address = %peer_addr,
                            error = %e,
                            "Failed to query peer status"
                        );
                        None
                    }
                }
            });
        }

        // Wait for all queries to complete
        let results = futures::future::join_all(connection_futures).await;

        for status in results.into_iter().flatten() {
            peer_statuses.push(status);
        }

        Ok(peer_statuses)
    }

    /// Query a single peer for cluster status
    async fn query_peer_status(peer_addr: &str, local_node_id: &str) -> Result<PeerStatus> {
        // Connect to peer
        let mut stream = timeout(Duration::from_secs(10), TcpStream::connect(peer_addr))
            .await
            .map_err(|_| Error::Backend("Connection timeout".to_string()))?
            .map_err(|e| Error::Backend(format!("Failed to connect to peer: {}", e)))?;

        // Send Hello handshake first (required by ReplicationServer)
        let hello = ReplicationMessage::Hello {
            cluster_node_id: local_node_id.to_string(),
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

        // Send GetClusterStatus request
        let request = ReplicationMessage::GetClusterStatus;

        Self::send_message(&mut stream, &request).await?;

        // Receive ClusterStatusResponse
        let response = Self::receive_message(&mut stream).await?;

        match response {
            ReplicationMessage::ClusterStatusResponse {
                node_id,
                log_index,
                max_vector_clock,
                num_tenants,
                num_repos,
                last_update_timestamp_ms,
                known_peers,
                tenant_repos,
                storage_size_bytes: _,
            } => {
                info!(
                    peer_id = %node_id,
                    log_index = log_index,
                    total_tenants = num_tenants,
                    total_repos = num_repos,
                    tenant_repo_pairs = tenant_repos.len(),
                    "Received cluster status from peer"
                );

                Ok(PeerStatus {
                    node_id,
                    address: peer_addr.to_string(),
                    log_index,
                    vector_clock: max_vector_clock,
                    num_tenants,
                    num_repos,
                    last_update_timestamp_ms,
                    known_peers,
                    tenant_repos,
                })
            }
            _ => Err(Error::Backend(
                "Unexpected response to GetClusterStatus".to_string(),
            )),
        }
    }

    /// Phase 2: Calculate consensus log index from peer responses
    pub(super) fn calculate_consensus(
        &self,
        peer_statuses: &[PeerStatus],
    ) -> Result<ConsensusState> {
        info!(
            "Phase 2: Calculating consensus from {} peers",
            peer_statuses.len()
        );

        if peer_statuses.is_empty() {
            return Err(Error::Backend(
                "No peers available for consensus".to_string(),
            ));
        }

        // Collect all vector clocks
        let peer_clocks: Vec<VectorClock> = peer_statuses
            .iter()
            .map(|p| p.vector_clock.clone())
            .collect();

        // Calculate consensus vector clock (max of all clocks)
        let consensus_clock = self
            .conflict_resolver
            .calculate_consensus_vector_clock(&peer_clocks);

        // Calculate consensus log index (median of peer log indexes)
        let mut log_indexes: Vec<u64> = peer_statuses.iter().map(|p| p.log_index).collect();
        log_indexes.sort_unstable();

        let consensus_log_index = if log_indexes.len() % 2 == 0 {
            let mid = log_indexes.len() / 2;
            (log_indexes[mid - 1] + log_indexes[mid]) / 2
        } else {
            log_indexes[log_indexes.len() / 2]
        };

        info!(
            consensus_log_index = consensus_log_index,
            "Consensus calculated"
        );

        Ok(ConsensusState {
            log_index: consensus_log_index,
            vector_clock: consensus_clock,
        })
    }

    /// Phase 3: Select source peer with most up-to-date state
    pub(super) fn select_source_peer<'a>(
        &self,
        peer_statuses: &'a [PeerStatus],
        consensus: &ConsensusState,
    ) -> Result<&'a PeerStatus> {
        info!("Phase 3: Selecting source peer for catch-up");

        // Select peer with highest log index, breaking ties by timestamp
        let source_peer = peer_statuses
            .iter()
            .filter(|p| p.log_index >= consensus.log_index)
            .max_by_key(|p| (p.log_index, p.last_update_timestamp_ms))
            .ok_or_else(|| {
                Error::Backend("No peer found with log index >= consensus".to_string())
            })?;

        Ok(source_peer)
    }
}
