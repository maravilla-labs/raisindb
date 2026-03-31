//! Coordinator lifecycle management.
//!
//! Contains the start method and periodic sync loop.

use std::sync::Arc;

use tokio::time::{interval, Duration};
use tracing::{debug, error, info, trace, warn};

use super::types::CoordinatorError;
use super::ReplicationCoordinator;
use crate::config::ClusterConfig;

impl ReplicationCoordinator {
    /// Start the coordinator (connect to peers and start sync tasks)
    pub async fn start(
        self: Arc<Self>,
        cluster_config: ClusterConfig,
    ) -> Result<(), CoordinatorError> {
        info!(
            node_id = %self.cluster_node_id,
            "Starting replication coordinator"
        );

        // CRITICAL FIX: Initialize causal buffer with current storage vector clock
        // This ensures operations can be delivered after node restart
        // Without this, the buffer starts with empty VC and buffers all incoming operations
        let stats = self
            .storage
            .get_cluster_stats()
            .await
            .map_err(|e| CoordinatorError::Storage(e.to_string()))?;

        if !stats.max_vector_clock.is_empty() {
            let mut buffer = self.causal_buffer.write().await;
            *buffer = crate::causal_delivery::CausalDeliveryBuffer::new(
                stats.max_vector_clock.clone(),
                Some(10_000),
            );
            info!(
                vc_size = stats.max_vector_clock.as_map().len(),
                num_tenants = stats.num_tenants,
                num_repos = stats.num_repos,
                "Initialized causal buffer with aggregated vector clock from storage"
            );
        } else {
            info!("Causal buffer initialized with empty vector clock (fresh node)");
        }

        // Set up sync-on-connect callback
        let coordinator = self.clone();
        self.peer_manager
            .set_on_connected(move |peer_id: String| {
                let coord = coordinator.clone();
                let pid = peer_id.clone();

                // Spawn task to trigger sync without blocking the connection
                tokio::spawn(async move {
                    info!(peer_id = %pid, "Sync-on-connect: Triggering immediate sync");

                    let tenant_repos = coord.collect_tenant_repo_pairs(Some(&pid)).await;
                    info!(
                        peer_id = %pid,
                        num_pairs = tenant_repos.len(),
                        "Sync-on-connect: Syncing {} tenant/repo pairs",
                        tenant_repos.len()
                    );

                    for (tenant_id, repo_id) in &tenant_repos {
                        if let Err(e) = coord
                            .sync_tenant_repo_with_peer(&pid, tenant_id, repo_id)
                            .await
                        {
                            warn!(
                                peer_id = %pid,
                                tenant_id = %tenant_id,
                                repo_id = %repo_id,
                                error = %e,
                                "Sync-on-connect: Sync failed"
                            );
                        }
                    }

                    info!(peer_id = %pid, "Sync-on-connect: Completed");
                });
            })
            .await;

        // Connect to all configured peers
        for peer_config in &cluster_config.peers {
            self.peer_manager.add_peer(peer_config.clone()).await;

            let coordinator = self.clone();
            let peer_id = peer_config.node_id.clone();

            // Spawn connection task
            tokio::spawn(async move {
                if let Err(e) = coordinator.peer_manager.connect_to_peer(&peer_id).await {
                    info!(peer_id = %peer_id, error = %e, "Initial connection to peer failed, will retry");
                } else {
                    info!(peer_id = %peer_id, "Connected to peer");

                    info!(peer_id = %peer_id, "Triggering immediate sync after connection");
                    let tenant_repos = coordinator.collect_tenant_repo_pairs(Some(&peer_id)).await;

                    info!(
                        peer_id = %peer_id,
                        num_pairs = tenant_repos.len(),
                        "Syncing {} tenant/repo pairs",
                        tenant_repos.len()
                    );

                    for (tenant_id, repo_id) in &tenant_repos {
                        if let Err(e) = coordinator
                            .sync_tenant_repo_with_peer(&peer_id, tenant_id, repo_id)
                            .await
                        {
                            warn!(
                                peer_id = %peer_id,
                                tenant_id = %tenant_id,
                                repo_id = %repo_id,
                                error = %e,
                                "Initial sync failed after connection"
                            );
                        }
                    }
                }
            });
        }

        // Start periodic pull sync
        if self.sync_config.interval_seconds > 0 {
            let coordinator = self.clone();
            tokio::spawn(async move {
                coordinator.run_periodic_sync().await;
            });
        }

        // Start heartbeat monitor
        let peer_manager = self.peer_manager.clone();
        tokio::spawn(async move {
            peer_manager.start_heartbeat_monitor().await;
        });

        // Start TCP server to accept incoming peer connections
        let mut server = crate::tcp_server::ReplicationServer::new(
            self.clone(),
            cluster_config.clone(),
            self.storage.clone(),
        );

        // Add checkpoint provider if available
        if let Some(ref checkpoint_provider) = self.checkpoint_provider {
            server = server.with_checkpoint_provider(checkpoint_provider.clone());
            info!("CheckpointProvider configured for incoming catch-up requests");
        }

        let server = Arc::new(server);

        tokio::spawn(async move {
            if let Err(e) = server.start().await {
                error!(error = %e, "Replication server failed");
            }
        });

        Ok(())
    }

    /// Run periodic pull-based synchronization
    async fn run_periodic_sync(self: Arc<Self>) {
        let mut ticker = interval(Duration::from_secs(self.sync_config.interval_seconds));

        loop {
            ticker.tick().await;

            trace!("Running periodic sync");

            // Get all peer statuses
            let statuses = self.peer_manager.get_all_peer_status().await;

            for status in statuses {
                if !matches!(status.state, crate::ConnectionState::Connected) {
                    continue;
                }

                let coordinator = self.clone();
                let peer_id = status.peer_id.clone();

                tokio::spawn(async move {
                    debug!(peer_id = %peer_id, "Starting periodic sync with peer");

                    let tenant_repos = coordinator.collect_tenant_repo_pairs(Some(&peer_id)).await;

                    // Sync all tenant/repo pairs
                    for (tenant_id, repo_id) in &tenant_repos {
                        if let Err(e) = coordinator
                            .sync_tenant_repo_with_peer(&peer_id, tenant_id, repo_id)
                            .await
                        {
                            warn!(peer_id = %peer_id, tenant_id = %tenant_id, repo_id = %repo_id, error = %e, "Sync failed");
                        }
                    }
                });
            }
        }
    }
}
