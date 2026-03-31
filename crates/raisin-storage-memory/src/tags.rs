//! In-memory tag repository implementation

use crate::index_types::TagIndex;
use raisin_context::Tag;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_storage::TagRepository;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct InMemoryTagRepo {
    /// Map: (tenant_id, repo_id, tag_name) -> Tag
    tags: TagIndex,
    event_bus: Arc<dyn raisin_storage::EventBus>,
}

impl Default for InMemoryTagRepo {
    fn default() -> Self {
        Self {
            tags: Arc::new(RwLock::new(HashMap::new())),
            event_bus: Arc::new(raisin_storage::InMemoryEventBus::new()),
        }
    }
}

impl InMemoryTagRepo {
    pub fn new(event_bus: Arc<dyn raisin_storage::EventBus>) -> Self {
        Self {
            tags: Arc::new(RwLock::new(HashMap::new())),
            event_bus,
        }
    }
}

impl TagRepository for InMemoryTagRepo {
    async fn create_tag(
        &self,
        tenant_id: &str,
        repo_id: &str,
        tag_name: &str,
        revision: &HLC,
        created_by: &str,
        message: Option<String>,
        protected: bool,
    ) -> Result<Tag> {
        let tag = Tag {
            name: tag_name.to_string(),
            revision: *revision,
            created_at: chrono::Utc::now(),
            created_by: created_by.to_string(),
            message: message.clone(),
            protected,
        };

        let key = (
            tenant_id.to_string(),
            repo_id.to_string(),
            tag_name.to_string(),
        );
        let mut tags = self.tags.write().await;
        tags.insert(key, tag.clone());
        drop(tags);

        // Emit TagCreated event
        self.event_bus.publish(raisin_storage::Event::Repository(
            raisin_storage::RepositoryEvent {
                tenant_id: tenant_id.to_string(),
                repository_id: repo_id.to_string(),
                kind: raisin_storage::RepositoryEventKind::TagCreated,
                workspace: None,
                revision_id: Some(revision.to_string()),
                branch_name: None,
                tag_name: Some(tag_name.to_string()),
                message,
                actor: Some(created_by.to_string()),
                metadata: None,
            },
        ));

        Ok(tag)
    }

    async fn get_tag(&self, tenant_id: &str, repo_id: &str, tag_name: &str) -> Result<Option<Tag>> {
        let key = (
            tenant_id.to_string(),
            repo_id.to_string(),
            tag_name.to_string(),
        );
        let tags = self.tags.read().await;
        Ok(tags.get(&key).cloned())
    }

    async fn list_tags(&self, tenant_id: &str, repo_id: &str) -> Result<Vec<Tag>> {
        let tags = self.tags.read().await;
        let result: Vec<Tag> = tags
            .iter()
            .filter(|((t, r, _), _)| t == tenant_id && r == repo_id)
            .map(|(_, tag)| tag.clone())
            .collect();
        Ok(result)
    }

    async fn delete_tag(&self, tenant_id: &str, repo_id: &str, tag_name: &str) -> Result<bool> {
        let key = (
            tenant_id.to_string(),
            repo_id.to_string(),
            tag_name.to_string(),
        );
        let mut tags = self.tags.write().await;

        // Check if tag exists and if it's protected
        if let Some(tag) = tags.get(&key) {
            if tag.protected {
                return Err(raisin_error::Error::Forbidden(format!(
                    "Cannot delete protected tag '{}'",
                    tag_name
                )));
            }
        }

        let deleted = tags.remove(&key).is_some();
        drop(tags);

        if deleted {
            // Emit TagDeleted event
            self.event_bus.publish(raisin_storage::Event::Repository(
                raisin_storage::RepositoryEvent {
                    tenant_id: tenant_id.to_string(),
                    repository_id: repo_id.to_string(),
                    kind: raisin_storage::RepositoryEventKind::TagDeleted,
                    workspace: None,
                    revision_id: None,
                    branch_name: None,
                    tag_name: Some(tag_name.to_string()),
                    message: None,
                    actor: None,
                    metadata: None,
                },
            ));
        }

        Ok(deleted)
    }
}
