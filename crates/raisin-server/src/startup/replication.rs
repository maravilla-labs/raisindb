//! Replication setup for cluster mode.
//!
//! This module handles discovery of tenants/repos for replication
//! and initialization of the replication coordinator.

#[cfg(feature = "storage-rocksdb")]
use raisin_rocksdb::RocksDBStorage;
#[cfg(feature = "storage-rocksdb")]
use std::sync::Arc;

use crate::config;

/// Start the replication coordinator if cluster mode is configured.
///
/// Returns `Some(coordinator)` if replication was successfully started,
/// or `None` if not configured or if startup failed.
#[cfg(feature = "storage-rocksdb")]
pub async fn start_replication_coordinator(
    storage: &Arc<RocksDBStorage>,
    cluster_node_id: Option<&str>,
    replication_port: Option<u16>,
    replication_peers: &[config::ReplicationPeer],
) -> Option<Arc<raisin_replication::ReplicationCoordinator>> {
    use raisin_replication::{ClusterConfig, ConnectionConfig, PeerConfig, SyncConfig};

    let (node_id, repl_port) = match (cluster_node_id, replication_port) {
        (Some(node_id), Some(port)) => (node_id, port),
        _ => {
            if cluster_node_id.is_some() || replication_port.is_some() {
                tracing::warn!(
                    "Incomplete replication config - need both cluster-node-id AND replication-port"
                );
            }
            tracing::info!("Replication not configured - running in standalone mode");
            return None;
        }
    };

    tracing::info!("Starting cluster replication for node: {}", node_id);

    let peer_configs: Vec<PeerConfig> = replication_peers
        .iter()
        .map(|peer| PeerConfig::new(&peer.peer_id, &peer.address).with_port(peer.port))
        .collect();

    tracing::info!("Configured {} replication peers", peer_configs.len());
    for peer in &peer_configs {
        tracing::info!("  Peer: {} at {}:{}", peer.node_id, peer.host, peer.port);
    }

    let sync_pairs = discover_sync_tenants(storage).await.unwrap_or_else(|e| {
        tracing::warn!(error = %e, "Falling back to default sync tenant list");
        Vec::new()
    });

    let sync_tenants = if sync_pairs.is_empty() {
        vec![("default".to_string(), "default".to_string())]
    } else {
        tracing::info!(
            sync_pairs = sync_pairs.len(),
            "Discovered {} tenant/repository pairs for replication",
            sync_pairs.len()
        );
        sync_pairs
    };

    let cluster_config = ClusterConfig {
        node_id: node_id.to_string(),
        replication_port: repl_port,
        bind_address: "0.0.0.0".to_string(),
        peers: peer_configs,
        sync: SyncConfig {
            interval_seconds: 5,
            batch_size: 1000,
            realtime_push: true,
            compression: "none".to_string(),
            compression_level: 0,
            retry: Default::default(),
        },
        connection: ConnectionConfig {
            heartbeat_interval_seconds: 30,
            connect_timeout_seconds: 10,
            read_timeout_seconds: 30,
            write_timeout_seconds: 30,
            max_connections_per_peer: 4,
            keepalive_seconds: 60,
        },
        sync_tenants,
    };

    match raisin_rocksdb::replication::start_replication(storage.clone(), cluster_config).await {
        Ok(coordinator) => {
            tracing::info!("Replication coordinator started successfully");
            tracing::info!("   TCP server listening on port {}", repl_port);
            Some(coordinator)
        }
        Err(e) => {
            tracing::error!("Failed to start replication: {}", e);
            tracing::warn!("Server will continue in STANDALONE mode");
            None
        }
    }
}

/// Discover all tenant/repository pairs for replication sync.
///
/// This scans existing tenants and repositories, plus any extra pairs
/// specified via environment variable RAISIN_CLUSTER_SYNC_EXTRA_REPOS.
#[cfg(feature = "storage-rocksdb")]
pub async fn discover_sync_tenants(
    storage: &Arc<RocksDBStorage>,
) -> Result<Vec<(String, String)>, raisin_error::Error> {
    use raisin_storage::{RegistryRepository, Storage};
    use std::collections::HashSet;

    let registry = Storage::registry(storage.as_ref());
    let tenants = registry.list_tenants().await?;
    let mut seen = HashSet::new();
    let mut pairs = Vec::new();

    for tenant in tenants {
        let tenant_id = tenant.tenant_id;

        let registry_pair = (tenant_id.clone(), "_registry".to_string());
        if seen.insert(registry_pair.clone()) {
            pairs.push(registry_pair);
        }

        match raisin_rocksdb::management::list_repositories(storage, &tenant_id).await {
            Ok(repos) => {
                for repo_id in repos {
                    let pair = (tenant_id.clone(), repo_id);
                    if seen.insert(pair.clone()) {
                        pairs.push(pair);
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    tenant_id = %tenant_id,
                    error = %e,
                    "Failed to enumerate repositories for tenant during sync discovery"
                );
            }
        }
    }

    // Handle extra sync pairs from environment
    if let Ok(extra) = std::env::var("RAISIN_CLUSTER_SYNC_EXTRA_REPOS") {
        if !extra.trim().is_empty() {
            tracing::info!(
                extra_sync = %extra,
                "Applying extra tenant/repo sync pairs from environment"
            );
        }
        for entry in extra.split(',') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }

            match entry.split_once(':') {
                Some((tenant, repo)) if !tenant.is_empty() && !repo.is_empty() => {
                    let pair = (tenant.to_string(), repo.to_string());
                    if seen.insert(pair.clone()) {
                        tracing::info!(
                            tenant_id = %pair.0,
                            repo_id = %pair.1,
                            "Adding extra sync tenant/repo pair from environment"
                        );
                        pairs.push(pair);
                    }
                }
                _ => tracing::warn!(
                    entry = %entry,
                    "Invalid RAISIN_CLUSTER_SYNC_EXTRA_REPOS entry (expected tenant:repo)"
                ),
            }
        }
    }

    // Always include the social feed demo repository so replication picks up the example app
    let social_feed_pair = ("default".to_string(), "social_feed_demo".to_string());
    if seen.insert(social_feed_pair.clone()) {
        tracing::info!(
            tenant_id = %social_feed_pair.0,
            repo_id = %social_feed_pair.1,
            "Ensuring social_feed_demo repository participates in cluster sync"
        );
        pairs.push(social_feed_pair);
    }

    Ok(pairs)
}
