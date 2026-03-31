//! Peer query and metrics methods for the replication coordinator.
//!
//! Contains tenant/repo pair collection, peer cluster snapshot queries,
//! sync statistics, and replication metrics.

use std::collections::HashSet;

use tracing::{info, warn};

use super::types::{CoordinatorError, PeerClusterSnapshot, SyncStats};
use super::ReplicationCoordinator;
use crate::metrics::ReplicationMetrics;
use crate::tcp_protocol::ReplicationMessage;

impl ReplicationCoordinator {
    /// Build a deduplicated list of tenant/repository pairs that should be synchronized
    pub(crate) async fn collect_tenant_repo_pairs(
        &self,
        peer_id: Option<&str>,
    ) -> Vec<(String, String)> {
        let mut seen = HashSet::new();
        let mut pairs = Vec::new();

        fn push_unique(
            seen: &mut HashSet<(String, String)>,
            pairs: &mut Vec<(String, String)>,
            tenant: String,
            repo: String,
        ) {
            if seen.insert((tenant.clone(), repo.clone())) {
                pairs.push((tenant, repo));
            }
        }

        match self.storage.get_cluster_stats().await {
            Ok(stats) => {
                for (tenant_id, repo_id) in stats.tenant_repos {
                    push_unique(&mut seen, &mut pairs, tenant_id, repo_id);
                }
            }
            Err(e) => {
                warn!(
                    error = %e,
                    "Failed to get local cluster stats, falling back to configured sync list"
                );
                for (tenant_id, repo_id) in &self.sync_tenants {
                    push_unique(&mut seen, &mut pairs, tenant_id.clone(), repo_id.clone());
                }
            }
        }

        if let Some(peer) = peer_id {
            match self
                .peer_manager
                .send_request(peer, &ReplicationMessage::GetClusterStatus)
                .await
            {
                Ok(ReplicationMessage::ClusterStatusResponse { tenant_repos, .. }) => {
                    for (tenant_id, repo_id) in tenant_repos {
                        push_unique(&mut seen, &mut pairs, tenant_id, repo_id);
                    }
                }
                Ok(other) => {
                    warn!(peer_id = %peer, "Unexpected cluster status response: {:?}", other);
                }
                Err(e) => {
                    warn!(
                        peer_id = %peer,
                        error = %e,
                        "Failed to fetch peer cluster status for tenant discovery"
                    );
                }
            }
        }

        if pairs.is_empty() {
            for (tenant_id, repo_id) in &self.sync_tenants {
                push_unique(&mut seen, &mut pairs, tenant_id.clone(), repo_id.clone());
            }
        }

        pairs
    }

    /// Query all connected peers for their cluster status snapshots
    pub async fn get_peer_cluster_snapshots(&self) -> Vec<PeerClusterSnapshot> {
        let peer_statuses = self.peer_manager.get_all_peer_status().await;
        let mut snapshots = Vec::new();

        for status in peer_statuses {
            if !matches!(status.state, crate::ConnectionState::Connected) {
                continue;
            }

            match self
                .peer_manager
                .send_request(&status.peer_id, &ReplicationMessage::GetClusterStatus)
                .await
            {
                Ok(ReplicationMessage::ClusterStatusResponse {
                    node_id,
                    log_index,
                    max_vector_clock,
                    num_tenants,
                    num_repos,
                    tenant_repos,
                    ..
                }) => {
                    info!(
                        peer_id = %status.peer_id,
                        node_id = %node_id,
                        log_index = log_index,
                        total_tenants = num_tenants,
                        total_repos = num_repos,
                        "Collected cluster status snapshot from peer"
                    );

                    snapshots.push(PeerClusterSnapshot {
                        configured_peer_id: status.peer_id.clone(),
                        node_id,
                        log_index,
                        vector_clock: max_vector_clock,
                        num_tenants,
                        num_repos,
                        tenant_repos,
                    });
                }
                Ok(other) => {
                    warn!(
                        peer_id = %status.peer_id,
                        message = ?other,
                        "Unexpected response to GetClusterStatus from peer"
                    );
                }
                Err(e) => {
                    warn!(
                        peer_id = %status.peer_id,
                        error = %e,
                        "Failed to fetch cluster status from peer"
                    );
                }
            }
        }

        snapshots
    }

    /// Get sync statistics
    pub async fn get_sync_stats(&self) -> SyncStats {
        let peer_statuses = self.peer_manager.get_all_peer_status().await;

        let connected_peers = peer_statuses
            .iter()
            .filter(|s| matches!(s.state, crate::ConnectionState::Connected))
            .count();

        let disconnected_peers = peer_statuses
            .iter()
            .filter(|s| !matches!(s.state, crate::ConnectionState::Connected))
            .count();

        SyncStats {
            cluster_node_id: self.cluster_node_id.clone(),
            total_peers: peer_statuses.len(),
            connected_peers,
            disconnected_peers,
        }
    }

    /// Get comprehensive replication metrics
    ///
    /// This provides a snapshot of all replication activity and performance.
    ///
    /// # Returns
    /// ReplicationMetrics with current state and performance data
    pub async fn get_metrics(&self) -> ReplicationMetrics {
        let sync_stats = self.get_sync_stats().await;

        // Calculate replication lag (would need peer vector clocks in real implementation)
        let replication_lag_ops = 0; // Placeholder - would compare local VC with peer VCs

        ReplicationMetrics {
            operations_pushed: self.metrics.operations_pushed.get(),
            operations_received: self.metrics.operations_received.get(),
            operations_applied: self.metrics.operations_applied.get(),
            operations_failed: self.metrics.operations_failed.get(),
            sync_cycles: self.metrics.sync_cycles.get(),
            avg_sync_duration_ms: self.metrics.sync_duration.avg_ms(),
            replication_lag_ops,
            catch_up_triggered: self.metrics.catch_up_triggered.get(),
            active_peers: sync_stats.connected_peers,
            total_peers: sync_stats.total_peers,
            conflicts_detected: self.metrics.conflicts_detected.get(),
            operations_skipped: self.metrics.operations_skipped.get(),
            p99_sync_latency_ms: self.metrics.sync_duration.percentile(99.0).as_millis() as u64,
            timestamp: crate::metrics::current_timestamp_ms(),
        }
    }
}
