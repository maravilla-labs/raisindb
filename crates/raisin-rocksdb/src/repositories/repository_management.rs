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
        let prefix = keys::KeyBuilder::new().push("repos").build_prefix();

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
