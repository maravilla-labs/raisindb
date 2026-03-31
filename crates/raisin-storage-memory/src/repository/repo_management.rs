//! In-memory repository management implementation.

use raisin_context::{RepositoryConfig, RepositoryInfo};
use raisin_error::Result;
use raisin_storage::RepositoryManagementRepository;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// In-memory repository management
#[derive(Clone)]
pub struct InMemoryRepositoryManagement {
    /// repositories: key = "{tenant_id}/{repo_id}" -> RepositoryInfo
    repositories: Arc<RwLock<HashMap<String, RepositoryInfo>>>,
    event_bus: Arc<dyn raisin_storage::EventBus>,
}

impl Default for InMemoryRepositoryManagement {
    fn default() -> Self {
        Self {
            repositories: Arc::new(RwLock::new(HashMap::new())),
            event_bus: Arc::new(raisin_storage::InMemoryEventBus::new()),
        }
    }
}

impl InMemoryRepositoryManagement {
    pub fn new(event_bus: Arc<dyn raisin_storage::EventBus>) -> Self {
        Self {
            repositories: Arc::new(RwLock::new(HashMap::new())),
            event_bus,
        }
    }

    fn make_key(tenant_id: &str, repo_id: &str) -> String {
        format!("{}/{}", tenant_id, repo_id)
    }
}

impl RepositoryManagementRepository for InMemoryRepositoryManagement {
    async fn create_repository(
        &self,
        tenant_id: &str,
        repo_id: &str,
        config: RepositoryConfig,
    ) -> Result<RepositoryInfo> {
        let key = Self::make_key(tenant_id, repo_id);
        let mut repos = self.repositories.write().await;

        if repos.contains_key(&key) {
            return Err(raisin_error::Error::Conflict(format!(
                "Repository {}/{} already exists",
                tenant_id, repo_id
            )));
        }

        let info = RepositoryInfo {
            tenant_id: tenant_id.to_string(),
            repo_id: repo_id.to_string(),
            created_at: chrono::Utc::now(),
            branches: vec![config.default_branch.clone()],
            config: config.clone(),
        };

        repos.insert(key, info.clone());

        // Drop the write lock before publishing event
        drop(repos);

        // Publish RepositoryCreated event
        let event = raisin_storage::Event::Repository(raisin_storage::RepositoryEvent {
            tenant_id: tenant_id.to_string(),
            repository_id: repo_id.to_string(),
            kind: raisin_storage::RepositoryEventKind::Created,
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
        let key = Self::make_key(tenant_id, repo_id);
        let repos = self.repositories.read().await;
        Ok(repos.get(&key).cloned())
    }

    async fn list_repositories(&self) -> Result<Vec<RepositoryInfo>> {
        let repos = self.repositories.read().await;
        Ok(repos.values().cloned().collect())
    }

    async fn list_repositories_for_tenant(&self, tenant_id: &str) -> Result<Vec<RepositoryInfo>> {
        let repos = self.repositories.read().await;
        Ok(repos
            .values()
            .filter(|info| info.tenant_id == tenant_id)
            .cloned()
            .collect())
    }

    async fn delete_repository(&self, tenant_id: &str, repo_id: &str) -> Result<bool> {
        let key = Self::make_key(tenant_id, repo_id);
        let mut repos = self.repositories.write().await;
        let deleted = repos.remove(&key).is_some();
        drop(repos);

        if deleted {
            // Emit RepositoryDeleted event
            self.event_bus.publish(raisin_storage::Event::Repository(
                raisin_storage::RepositoryEvent {
                    tenant_id: tenant_id.to_string(),
                    repository_id: repo_id.to_string(),
                    kind: raisin_storage::RepositoryEventKind::Deleted,
                    workspace: None,
                    revision_id: None,
                    branch_name: None,
                    tag_name: None,
                    message: None,
                    actor: None,
                    metadata: None,
                },
            ));
        }

        Ok(deleted)
    }

    async fn repository_exists(&self, tenant_id: &str, repo_id: &str) -> Result<bool> {
        let key = Self::make_key(tenant_id, repo_id);
        let repos = self.repositories.read().await;
        Ok(repos.contains_key(&key))
    }

    async fn update_repository_config(
        &self,
        tenant_id: &str,
        repo_id: &str,
        config: RepositoryConfig,
    ) -> Result<()> {
        let key = Self::make_key(tenant_id, repo_id);
        let mut repos = self.repositories.write().await;

        if let Some(info) = repos.get_mut(&key) {
            info.config = config;
            drop(repos);

            // Emit RepositoryUpdated event
            self.event_bus.publish(raisin_storage::Event::Repository(
                raisin_storage::RepositoryEvent {
                    tenant_id: tenant_id.to_string(),
                    repository_id: repo_id.to_string(),
                    kind: raisin_storage::RepositoryEventKind::Updated,
                    workspace: None,
                    revision_id: None,
                    branch_name: None,
                    tag_name: None,
                    message: None,
                    actor: None,
                    metadata: None,
                },
            ));

            Ok(())
        } else {
            Err(raisin_error::Error::NotFound(format!(
                "Repository {}/{}",
                tenant_id, repo_id
            )))
        }
    }
}
