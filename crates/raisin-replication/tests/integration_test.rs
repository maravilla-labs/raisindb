//! Integration tests for TCP replication
//!
//! These tests verify that the complete replication system works end-to-end.

use raisin_replication::{
    ClusterConfig, ConnectionConfig, OperationLogStorage, PeerConfig, ReplicationCoordinator,
    ReplicationServer, StorageError, SyncConfig, VectorClock,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Mock storage implementation for testing
struct MockStorage {
    operations: Arc<Mutex<HashMap<String, Vec<raisin_replication::Operation>>>>,
    vector_clocks: Arc<Mutex<HashMap<String, VectorClock>>>,
}

impl MockStorage {
    fn new() -> Self {
        Self {
            operations: Arc::new(Mutex::new(HashMap::new())),
            vector_clocks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn key(tenant_id: &str, repo_id: &str) -> String {
        format!("{}:{}", tenant_id, repo_id)
    }
}

#[async_trait::async_trait]
impl OperationLogStorage for MockStorage {
    async fn get_operations_since(
        &self,
        tenant_id: &str,
        repo_id: &str,
        _since_vc: &VectorClock,
        limit: usize,
    ) -> Result<Vec<raisin_replication::Operation>, StorageError> {
        let key = Self::key(tenant_id, repo_id);
        let ops = self.operations.lock().await;
        Ok(ops
            .get(&key)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .take(limit)
            .collect())
    }

    async fn put_operations_batch(
        &self,
        ops: &[raisin_replication::Operation],
    ) -> Result<(), StorageError> {
        if ops.is_empty() {
            return Ok(());
        }

        let key = Self::key(&ops[0].tenant_id, &ops[0].repo_id);
        let mut storage = self.operations.lock().await;
        storage.entry(key).or_default().extend_from_slice(ops);
        Ok(())
    }

    async fn get_vector_clock(
        &self,
        tenant_id: &str,
        repo_id: &str,
    ) -> Result<VectorClock, StorageError> {
        let key = Self::key(tenant_id, repo_id);
        let vcs = self.vector_clocks.lock().await;
        Ok(vcs.get(&key).cloned().unwrap_or_else(VectorClock::new))
    }

    async fn get_operations_for_node(
        &self,
        tenant_id: &str,
        repo_id: &str,
        node_id: &str,
        _since_seq: u64,
        limit: usize,
    ) -> Result<Vec<raisin_replication::Operation>, StorageError> {
        let key = Self::key(tenant_id, repo_id);
        let ops = self.operations.lock().await;
        Ok(ops
            .get(&key)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|op| op.cluster_node_id == node_id)
            .take(limit)
            .collect())
    }

    async fn get_cluster_stats(
        &self,
    ) -> Result<raisin_replication::ClusterStorageStats, StorageError> {
        let vcs = self.vector_clocks.lock().await;

        // Parse all keys to extract tenant/repo pairs
        let mut tenant_repos = Vec::new();
        let mut max_vc = VectorClock::new();

        for (key, vc) in vcs.iter() {
            if let Some((tenant_id, repo_id)) = key.split_once(':') {
                tenant_repos.push((tenant_id.to_string(), repo_id.to_string()));
                max_vc.merge(vc);
            }
        }

        let unique_tenants: std::collections::HashSet<_> =
            tenant_repos.iter().map(|(t, _)| t.clone()).collect();

        Ok(raisin_replication::ClusterStorageStats {
            max_vector_clock: max_vc,
            num_tenants: unique_tenants.len(),
            num_repos: tenant_repos.len(),
            tenant_repos,
        })
    }
}

#[tokio::test]
async fn test_coordinator_creation() {
    let config = ClusterConfig::single_node("test_node");
    let storage = Arc::new(MockStorage::new());

    let result = ReplicationCoordinator::new(config, storage);
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_server_startup() {
    // Create a test cluster config with a non-standard port to avoid conflicts
    let config = ClusterConfig {
        node_id: "test_server".to_string(),
        replication_port: 19001, // Use high port for testing
        bind_address: "127.0.0.1".to_string(),
        peers: vec![],
        sync: SyncConfig::default(),
        connection: ConnectionConfig::default(),
        sync_tenants: vec![("default".to_string(), "default".to_string())],
    };

    let storage = Arc::new(MockStorage::new());
    let coordinator =
        Arc::new(ReplicationCoordinator::new(config.clone(), storage.clone()).unwrap());
    let server = Arc::new(ReplicationServer::new(coordinator, config, storage));

    // Start server in background task
    let server_task = tokio::spawn(async move { server.start().await });

    // Give server time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Abort the server task (it runs forever)
    server_task.abort();
}

#[tokio::test]
async fn test_coordinator_stats() {
    let config = ClusterConfig::single_node("stats_test");
    let storage = Arc::new(MockStorage::new());

    let coordinator = ReplicationCoordinator::new(config, storage).unwrap();
    let stats = coordinator.get_sync_stats().await;

    assert_eq!(stats.cluster_node_id, "stats_test");
    assert_eq!(stats.total_peers, 0);
    assert_eq!(stats.connected_peers, 0);
}

#[tokio::test]
async fn test_coordinator_with_peers() {
    let peer1 = PeerConfig::new("peer1", "127.0.0.1");
    let peer2 = PeerConfig::new("peer2", "127.0.0.2");

    let mut config = ClusterConfig::single_node("node_with_peers");
    config.peers = vec![peer1, peer2];

    let storage = Arc::new(MockStorage::new());

    let coordinator = ReplicationCoordinator::new(config, storage).unwrap();
    let coordinator = Arc::new(coordinator);

    // Note: We don't actually start the coordinator here because it would try
    // to connect to non-existent peers. This test just verifies construction.
    let stats = coordinator.get_sync_stats().await;
    assert_eq!(stats.cluster_node_id, "node_with_peers");
}
