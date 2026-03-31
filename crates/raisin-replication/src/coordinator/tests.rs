#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    use crate::config::ClusterConfig;
    use crate::coordinator::traits::OperationLogStorage;
    use crate::coordinator::types::{ClusterStorageStats, StorageError};
    use crate::coordinator::ReplicationCoordinator;
    use crate::{Operation, VectorClock};
    use async_trait::async_trait;

    /// Mock storage for testing
    struct MockStorage {
        operations: Arc<Mutex<HashMap<String, Vec<Operation>>>>,
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

    #[async_trait]
    impl OperationLogStorage for MockStorage {
        async fn get_operations_since(
            &self,
            tenant_id: &str,
            repo_id: &str,
            _since_vc: &VectorClock,
            limit: usize,
        ) -> Result<Vec<Operation>, StorageError> {
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

        async fn put_operations_batch(&self, ops: &[Operation]) -> Result<(), StorageError> {
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
        ) -> Result<Vec<Operation>, StorageError> {
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

        async fn get_cluster_stats(&self) -> Result<ClusterStorageStats, StorageError> {
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

            Ok(ClusterStorageStats {
                max_vector_clock: max_vc,
                num_tenants: unique_tenants.len(),
                num_repos: tenant_repos.len(),
                tenant_repos,
            })
        }
    }

    #[tokio::test]
    async fn test_coordinator_creation() {
        let config = ClusterConfig::single_node("node1");
        let storage = Arc::new(MockStorage::new());

        let coordinator = ReplicationCoordinator::new(config, storage);
        assert!(coordinator.is_ok());
    }

    #[tokio::test]
    async fn test_sync_stats() {
        let config = ClusterConfig::single_node("node1");
        let storage = Arc::new(MockStorage::new());

        let coordinator = ReplicationCoordinator::new(config, storage).unwrap();
        let stats = coordinator.get_sync_stats().await;

        assert_eq!(stats.cluster_node_id, "node1");
        assert_eq!(stats.total_peers, 0);
        assert_eq!(stats.connected_peers, 0);
    }
}
