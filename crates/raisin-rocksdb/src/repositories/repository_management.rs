//! Repository management implementation

use crate::{cf, cf_handle, keys};
use raisin_context::{RepositoryConfig, RepositoryInfo};
use raisin_error::Result;
use raisin_events::EventBus;
use raisin_storage::RepositoryManagementRepository;
use rocksdb::DB;
use std::sync::Arc;

#[derive(Clone)]
pub struct RepositoryManagementRepositoryImpl {
    db: Arc<DB>,
    event_bus: Arc<dyn EventBus>,
    operation_capture: Option<Arc<crate::OperationCapture>>,
}

impl RepositoryManagementRepositoryImpl {
    pub fn new(db: Arc<DB>, event_bus: Arc<dyn EventBus>) -> Self {
        Self {
            db,
            event_bus,
            operation_capture: None,
        }
    }

    pub fn new_with_capture(
        db: Arc<DB>,
        event_bus: Arc<dyn EventBus>,
        operation_capture: Arc<crate::OperationCapture>,
    ) -> Self {
        Self {
            db,
            event_bus,
            operation_capture: Some(operation_capture),
        }
    }
}

impl RepositoryManagementRepository for RepositoryManagementRepositoryImpl {
    async fn create_repository(
        &self,
        tenant_id: &str,
        repo_id: &str,
        config: RepositoryConfig,
    ) -> Result<RepositoryInfo> {
        let info = RepositoryInfo {
            tenant_id: tenant_id.to_string(),
            repo_id: repo_id.to_string(),
            created_at: chrono::Utc::now(),
            branches: Vec::new(),
            config: config.clone(),
        };

        let key = keys::repository_key(tenant_id, repo_id);
        let value = rmp_serde::to_vec(&info)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        let cf = cf_handle(&self.db, cf::REGISTRY)?;
        self.db
            .put_cf(cf, key, value)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        // Capture operation for replication
        if let Some(ref capture) = self.operation_capture {
            if capture.is_enabled() {
                let _op = capture
                    .capture_update_repository(
                        tenant_id.to_string(),
                        repo_id.to_string(),
                        info.clone(),
                        "system".to_string(),
                    )
                    .await;
                // Ignore capture errors - don't fail repository creation if replication fails
            }
        }

        // Emit RepositoryCreated event to trigger NodeType initialization
        let event = raisin_events::Event::Repository(raisin_events::RepositoryEvent {
            tenant_id: tenant_id.to_string(),
            repository_id: repo_id.to_string(),
            kind: raisin_events::RepositoryEventKind::Created,
            workspace: None,
            revision_id: None,
            branch_name: Some(config.default_branch.clone()),
            tag_name: None,
            message: None,
            actor: None,
            metadata: None,
        });

        self.event_bus.publish(event);

        Ok(info)
    }

    async fn get_repository(
        &self,
        tenant_id: &str,
        repo_id: &str,
    ) -> Result<Option<RepositoryInfo>> {
        let key = keys::repository_key(tenant_id, repo_id);
        let cf = cf_handle(&self.db, cf::REGISTRY)?;

        match self.db.get_cf(cf, key) {
            Ok(Some(bytes)) => {
                let info = rmp_serde::from_slice(&bytes).map_err(|e| {
                    raisin_error::Error::storage(format!("Deserialization error: {}", e))
                })?;
                Ok(Some(info))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(raisin_error::Error::storage(e.to_string())),
        }
    }

    async fn list_repositories(&self) -> Result<Vec<RepositoryInfo>> {
        // Repository keys are laid out as `{tenant}\0repos\0{repo}` (see
        // `keys::repository_key`), i.e. tenant-FIRST. There is no global
        // `repos\0...` prefix to scan, so a prefix iterator over "repos"
        // matches nothing and silently returned an empty list (which broke
        // e.g. the startup builtin-package scan over all tenants). Instead,
        // scan the whole REGISTRY column family and filter by key shape.
        let cf = cf_handle(&self.db, cf::REGISTRY)?;
        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::Start);

        let mut repos = Vec::new();

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Only keys of the exact shape `{tenant}\0repos\0{repo}` are
            // repository entries; the REGISTRY CF also holds other records
            // (e.g. `tenants\0{tenant_id}`).
            let parts: Vec<&[u8]> = key.split(|b| *b == 0).collect();
            if parts.len() != 3 || parts[1] != b"repos" {
                continue;
            }

            let info: RepositoryInfo = rmp_serde::from_slice(&value).map_err(|e| {
                raisin_error::Error::storage(format!("Deserialization error: {}", e))
            })?;
            repos.push(info);
        }

        Ok(repos)
    }

    async fn list_repositories_for_tenant(&self, tenant_id: &str) -> Result<Vec<RepositoryInfo>> {
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push("repos")
            .build_prefix();

        let cf = cf_handle(&self.db, cf::REGISTRY)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        let mut repos = Vec::new();

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&prefix_clone) {
                break;
            }
            let info: RepositoryInfo = rmp_serde::from_slice(&value).map_err(|e| {
                raisin_error::Error::storage(format!("Deserialization error: {}", e))
            })?;
            repos.push(info);
        }

        Ok(repos)
    }

    async fn delete_repository(&self, tenant_id: &str, repo_id: &str) -> Result<bool> {
        let key = keys::repository_key(tenant_id, repo_id);
        let cf = cf_handle(&self.db, cf::REGISTRY)?;

        let exists = self
            .db
            .get_cf(cf, &key)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?
            .is_some();

        if exists {
            self.db
                .delete_cf(cf, key)
                .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn repository_exists(&self, tenant_id: &str, repo_id: &str) -> Result<bool> {
        Ok(self.get_repository(tenant_id, repo_id).await?.is_some())
    }

    async fn update_repository_config(
        &self,
        tenant_id: &str,
        repo_id: &str,
        config: RepositoryConfig,
    ) -> Result<()> {
        if let Some(mut info) = self.get_repository(tenant_id, repo_id).await? {
            info.config = config;

            let key = keys::repository_key(tenant_id, repo_id);
            let value = rmp_serde::to_vec(&info)
                .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

            let cf = cf_handle(&self.db, cf::REGISTRY)?;
            self.db
                .put_cf(cf, key, value)
                .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Capture operation for replication
            if let Some(ref capture) = self.operation_capture {
                if capture.is_enabled() {
                    let _op = capture
                        .capture_update_repository(
                            tenant_id.to_string(),
                            repo_id.to_string(),
                            info.clone(),
                            "system".to_string(),
                        )
                        .await;
                    // Ignore capture errors - don't fail update if replication fails
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_repo() -> RepositoryManagementRepositoryImpl {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = Arc::new(crate::open_db(temp_dir.path()).unwrap());
        // Keep tempdir alive for the test duration by leaking it (test-only).
        std::mem::forget(temp_dir);
        let event_bus = Arc::new(raisin_events::InMemoryEventBus::new());
        RepositoryManagementRepositoryImpl::new(db, event_bus)
    }

    /// Regression test: the global `list_repositories()` used to scan the
    /// prefix `repos\0`, but repository keys are `{tenant}\0repos\0{repo}`
    /// (tenant-first), so it always returned an empty list. That silently
    /// broke every all-tenant scan, most visibly the startup builtin-package
    /// auto-update which saw "0 repositories" and never reinstalled updated
    /// packages into existing repos.
    #[tokio::test]
    async fn test_list_repositories_returns_repos_across_tenants() {
        let repo_mgmt = make_repo();

        repo_mgmt
            .create_repository("tenant-a", "repo-1", RepositoryConfig::default())
            .await
            .unwrap();
        repo_mgmt
            .create_repository("tenant-a", "repo-2", RepositoryConfig::default())
            .await
            .unwrap();
        repo_mgmt
            .create_repository("tenant-b", "repo-3", RepositoryConfig::default())
            .await
            .unwrap();

        let all = repo_mgmt.list_repositories().await.unwrap();
        let mut pairs: Vec<(String, String)> = all
            .iter()
            .map(|r| (r.tenant_id.clone(), r.repo_id.clone()))
            .collect();
        pairs.sort();
        assert_eq!(
            pairs,
            vec![
                ("tenant-a".to_string(), "repo-1".to_string()),
                ("tenant-a".to_string(), "repo-2".to_string()),
                ("tenant-b".to_string(), "repo-3".to_string()),
            ],
            "global list_repositories must return every tenant's repositories"
        );

        // Per-tenant listing still scopes correctly.
        let tenant_a = repo_mgmt
            .list_repositories_for_tenant("tenant-a")
            .await
            .unwrap();
        assert_eq!(tenant_a.len(), 2);
        assert!(tenant_a.iter().all(|r| r.tenant_id == "tenant-a"));
    }
}
