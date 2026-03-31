// SPDX-License-Identifier: BSL-1.1

//! Event handler for automatic full-text index updates

use raisin_error::Result;
use raisin_events::{Event, EventHandler, NodeEvent, NodeEventKind};
use raisin_models::nodes::properties::schema::IndexType;
use raisin_storage::{
    FullTextIndexJob, FullTextJobStore, JobKind, NodeRepository, NodeTypeRepository,
    RepositoryManagementRepository, Storage, StorageScope,
};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Event handler that enqueues full-text indexing jobs when nodes change
///
/// This handler listens to node lifecycle events (create, update, delete) and
/// creates corresponding indexing jobs in the persistent job queue.
pub struct FullTextEventHandler<S: Storage> {
    storage: Arc<S>,
}

impl<S: Storage> FullTextEventHandler<S> {
    /// Creates a new FullTextEventHandler
    pub fn new(storage: Arc<S>) -> Self {
        Self { storage }
    }

    /// Fetches repository configuration for language settings
    async fn get_repository_languages(
        &self,
        tenant_id: &str,
        repo_id: &str,
    ) -> Result<(String, Vec<String>)> {
        let repo_info = self
            .storage
            .repository_management()
            .get_repository(tenant_id, repo_id)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!(
                    "Repository {}/{} not found",
                    tenant_id, repo_id
                ))
            })?;

        Ok((
            repo_info.config.default_language,
            repo_info.config.supported_languages,
        ))
    }

    /// Determines which properties should be indexed based on node type schema
    ///
    /// This performs a simplified schema resolution without full inheritance.
    /// For full inheritance support, this would need NodeTypeResolver from raisin-core.
    async fn resolve_properties_to_index(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        node_id: &str,
    ) -> Result<Option<Vec<String>>> {
        // Fetch the node to get its type
        let scope = StorageScope::new(tenant_id, repo_id, branch, workspace_id);
        let node = self
            .storage
            .nodes()
            .get(scope, node_id, None)
            .await?
            .ok_or_else(|| raisin_error::Error::NotFound(format!("Node {} not found", node_id)))?;

        // Fetch the node type definition
        let node_type = self
            .storage
            .node_types()
            .get(
                raisin_storage::scope::BranchScope::new(tenant_id, repo_id, branch),
                &node.node_type,
                None,
            )
            .await?;

        let node_type = match node_type {
            Some(nt) => nt,
            None => {
                tracing::warn!(
                    node_type = %node.node_type,
                    "Node type not found, will index all string properties"
                );
                return Ok(None);
            }
        };

        // Check if indexing is enabled
        if node_type.indexable == Some(false) {
            tracing::debug!(
                node_type = %node.node_type,
                "Node type has indexable=false, skipping fulltext indexing"
            );
            return Ok(Some(vec![])); // Empty list means don't index
        }

        // Check if Fulltext is in the allowed index types
        let index_types = node_type.index_types.as_ref();
        if let Some(types) = index_types {
            if !types.contains(&IndexType::Fulltext) {
                tracing::debug!(
                    node_type = %node.node_type,
                    index_types = ?types,
                    "Fulltext not in index_types, skipping"
                );
                return Ok(Some(vec![]));
            }
        }

        // Extract properties marked for fulltext indexing
        let properties = node_type.properties.as_ref();
        if let Some(props) = properties {
            let fulltext_props: Vec<String> = props
                .iter()
                .filter_map(|prop| {
                    let name = prop.name.as_ref()?;
                    let indexes = prop.index.as_ref()?;
                    if indexes.contains(&IndexType::Fulltext) {
                        Some(name.clone())
                    } else {
                        None
                    }
                })
                .collect();

            if !fulltext_props.is_empty() {
                tracing::debug!(
                    node_type = %node.node_type,
                    properties = ?fulltext_props,
                    "Resolved properties for fulltext indexing"
                );
                return Ok(Some(fulltext_props));
            }
        }

        // No specific properties marked, fall back to default behavior
        Ok(None)
    }

    /// Handles node creation/update events
    async fn handle_node_change(&self, node_event: &NodeEvent) -> Result<()> {
        let (default_language, supported_languages) = self
            .get_repository_languages(&node_event.tenant_id, &node_event.repository_id)
            .await?;

        // Resolve which properties should be indexed based on schema
        let properties_to_index = self
            .resolve_properties_to_index(
                &node_event.tenant_id,
                &node_event.repository_id,
                &node_event.branch,
                &node_event.workspace_id,
                &node_event.node_id,
            )
            .await?;

        // If properties_to_index is Some(vec![]), skip indexing entirely
        if let Some(ref props) = properties_to_index {
            if props.is_empty() {
                tracing::debug!(
                    node_id = %node_event.node_id,
                    "Node type not configured for fulltext indexing, skipping job"
                );
                return Ok(());
            }
        }

        let job = FullTextIndexJob {
            job_id: uuid::Uuid::new_v4().to_string(),
            kind: JobKind::AddNode,
            tenant_id: node_event.tenant_id.clone(),
            repo_id: node_event.repository_id.clone(),
            workspace_id: node_event.workspace_id.clone(),
            branch: node_event.branch.clone(),
            revision: node_event.revision,
            node_id: Some(node_event.node_id.clone()),
            source_branch: None,
            default_language,
            supported_languages,
            properties_to_index,
        };

        self.storage.fulltext_job_store().enqueue(&job)?;

        tracing::debug!(
            job_id = %job.job_id,
            node_id = %node_event.node_id,
            revision = ?node_event.revision,
            properties = ?job.properties_to_index,
            "Enqueued indexing job for node change"
        );

        Ok(())
    }

    /// Handles node deletion events
    async fn handle_node_delete(&self, node_event: &NodeEvent) -> Result<()> {
        let (default_language, supported_languages) = self
            .get_repository_languages(&node_event.tenant_id, &node_event.repository_id)
            .await?;

        let job = FullTextIndexJob {
            job_id: uuid::Uuid::new_v4().to_string(),
            kind: JobKind::DeleteNode,
            tenant_id: node_event.tenant_id.clone(),
            repo_id: node_event.repository_id.clone(),
            workspace_id: node_event.workspace_id.clone(),
            branch: node_event.branch.clone(),
            revision: node_event.revision,
            node_id: Some(node_event.node_id.clone()),
            source_branch: None,
            default_language,
            supported_languages,
            properties_to_index: None, // Not needed for deletion
        };

        self.storage.fulltext_job_store().enqueue(&job)?;

        tracing::debug!(
            job_id = %job.job_id,
            node_id = %node_event.node_id,
            "Enqueued deletion job for node"
        );

        Ok(())
    }
}

impl<S: Storage> EventHandler for FullTextEventHandler<S> {
    fn name(&self) -> &str {
        "fulltext_indexer"
    }

    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let node_event = match event {
                Event::Node(node_event) => node_event,
                _ => return Ok(()),
            };

            let result = match &node_event.kind {
                NodeEventKind::Created | NodeEventKind::Updated => {
                    self.handle_node_change(node_event).await
                }
                NodeEventKind::Deleted => self.handle_node_delete(node_event).await,
                _ => return Ok(()),
            };

            if let Err(e) = &result {
                tracing::error!(
                    error = %e,
                    node_id = %node_event.node_id,
                    event_kind = ?node_event.kind,
                    "Failed to enqueue indexing job"
                );
            }

            result.map_err(|e| anyhow::anyhow!(e))
        })
    }
}
