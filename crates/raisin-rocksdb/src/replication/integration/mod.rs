//! Integration between raisin-replication and RocksDB storage
//!
//! This module implements the OperationLogStorage trait from raisin-replication
//! for RocksDB, allowing the replication coordinator to work with RocksDB's operation log.

mod catchup;
mod checkpoint_ingestor;
mod oplog_storage;
mod startup;

use std::sync::Arc;

use crate::repositories::RegistryRepositoryImpl;
use crate::RocksDBStorage;
use raisin_storage::RegistryRepository;
use raisin_storage::Storage;

pub use checkpoint_ingestor::RocksDbCheckpointIngestor;
pub use oplog_storage::RocksDbOperationLogStorage;
pub use startup::start_replication;

/// Enumerate all tenant/repo pairs that exist in the database
///
/// This scans the REGISTRY column family to discover all tenants and their repositories,
/// ensuring catch-up protocol checks ALL data instead of only configured tenant/repo pairs.
///
/// **Important**: This includes the special `_registry` pseudo-repository for each tenant,
/// which stores tenant-level operations (UpdateTenant, UpdateDeployment, etc.).
async fn enumerate_all_tenant_repos(
    db: &Arc<RocksDBStorage>,
) -> Result<Vec<(String, String)>, raisin_error::Error> {
    let registry_repo = RegistryRepositoryImpl::new(db.db().clone(), db.event_bus().clone());

    // Get all tenants from REGISTRY CF
    let tenants = registry_repo.list_tenants().await?;
    let num_tenants = tenants.len();

    let mut tenant_repos = Vec::new();

    // For each tenant, enumerate all repositories
    for tenant in tenants {
        let tenant_id = &tenant.tenant_id;

        // CRITICAL: Always include _registry pseudo-repository for tenant-level operations
        // This ensures we detect operations like UpdateTenant, UpdateDeployment, etc.
        // which are captured during server initialization.
        tenant_repos.push((tenant_id.clone(), "_registry".to_string()));

        // Then enumerate all real repositories for this tenant
        match crate::management::list_repositories(db, tenant_id).await {
            Ok(repos) => {
                for repo_id in repos {
                    tenant_repos.push((tenant_id.clone(), repo_id));
                }
            }
            Err(e) => {
                tracing::warn!(
                    tenant_id = %tenant_id,
                    error = %e,
                    "Failed to list repositories for tenant during enumeration"
                );
            }
        }
    }

    tracing::debug!(
        count = tenant_repos.len(),
        num_tenants = num_tenants,
        "Enumerated tenant/repo pairs from database (includes _registry for each tenant)"
    );

    Ok(tenant_repos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_replication::{ClusterConfig, OperationLogStorage};
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_rocksdb_storage_adapter() {
        let temp_dir = TempDir::new().unwrap();
        let db = Arc::new(RocksDBStorage::new(temp_dir.path()).unwrap());
        let storage = RocksDbOperationLogStorage::new(db);

        // Test get_vector_clock on empty database
        let vc = storage.get_vector_clock("tenant1", "repo1").await.unwrap();
        assert!(vc.is_empty());
    }

    #[tokio::test]
    async fn test_start_replication() {
        let temp_dir = TempDir::new().unwrap();
        let db = Arc::new(RocksDBStorage::new(temp_dir.path()).unwrap());
        let cluster_config = ClusterConfig::single_node("test_node");

        let result = start_replication(db, cluster_config).await;
        assert!(result.is_ok());
    }
}
