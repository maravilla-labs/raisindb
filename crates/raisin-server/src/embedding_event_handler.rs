//! Event handler for automatically enqueueing embedding jobs
//!
//! This handler follows the same pattern as FullTextEventHandler,
//! listening to node lifecycle events and creating embedding jobs.

use raisin_embeddings::models::{EmbeddingJob, EmbeddingJobKind};
use raisin_embeddings::EmbeddingJobStore;
use raisin_error::Result;
use raisin_events::{
    Event, EventHandler, NodeEvent, NodeEventKind, RepositoryEvent, RepositoryEventKind,
};
use raisin_rocksdb::RocksDBEmbeddingJobStore;
use raisin_storage::Storage;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Event handler that enqueues embedding generation jobs when nodes change
///
/// This handler listens to:
/// - Node lifecycle events (create, update, delete)
/// - Repository events (branch creation)
/// and creates corresponding embedding jobs in the persistent job queue.
pub struct EmbeddingEventHandler<S: Storage> {
    storage: Arc<S>,
    job_store: Arc<RocksDBEmbeddingJobStore>,
}

impl<S: Storage> EmbeddingEventHandler<S> {
    /// Creates a new EmbeddingEventHandler
    pub fn new(storage: Arc<S>, job_store: Arc<RocksDBEmbeddingJobStore>) -> Self {
        Self { storage, job_store }
    }

    /// Handles node creation/update events
    async fn handle_node_change(&self, node_event: &NodeEvent) -> Result<()> {
        let job = EmbeddingJob {
            job_id: uuid::Uuid::new_v4().to_string(),
            kind: EmbeddingJobKind::AddNode,
            tenant_id: node_event.tenant_id.clone(),
            repo_id: node_event.repository_id.clone(),
            branch: node_event.branch.clone(),
            workspace_id: node_event.workspace_id.clone(),
            revision: node_event.revision,
            node_id: Some(node_event.node_id.clone()),
            source_branch: None,
            created_at: chrono::Utc::now(),
        };

        self.job_store.as_ref().enqueue(&job)?;

        tracing::debug!(
            job_id = %job.job_id,
            node_id = %node_event.node_id,
            revision = node_event.revision,
            "Enqueued embedding job for node change"
        );

        Ok(())
    }

    /// Handles node deletion events
    async fn handle_node_delete(&self, node_event: &NodeEvent) -> Result<()> {
        let job = EmbeddingJob {
            job_id: uuid::Uuid::new_v4().to_string(),
            kind: EmbeddingJobKind::DeleteNode,
            tenant_id: node_event.tenant_id.clone(),
            repo_id: node_event.repository_id.clone(),
            branch: node_event.branch.clone(),
            workspace_id: node_event.workspace_id.clone(),
            revision: node_event.revision,
            node_id: Some(node_event.node_id.clone()),
            source_branch: None,
            created_at: chrono::Utc::now(),
        };

        self.job_store.as_ref().enqueue(&job)?;

        tracing::debug!(
            job_id = %job.job_id,
            node_id = %node_event.node_id,
            "Enqueued deletion job for node embedding"
        );

        Ok(())
    }

    /// Handles branch creation events
    async fn handle_branch_created(&self, repo_event: &RepositoryEvent) -> Result<()> {
        let branch_name = repo_event.branch_name.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("BranchCreated event missing branch_name".to_string())
        })?;

        // Extract source branch from metadata (if provided)
        let source_branch = repo_event
            .metadata
            .as_ref()
            .and_then(|m| m.get("source_branch"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let job = EmbeddingJob {
            job_id: uuid::Uuid::new_v4().to_string(),
            kind: EmbeddingJobKind::BranchCreated,
            tenant_id: repo_event.tenant_id.clone(),
            repo_id: repo_event.repository_id.clone(),
            branch: branch_name.clone(),
            workspace_id: repo_event
                .workspace
                .clone()
                .unwrap_or_else(|| "default".to_string()),
            revision: raisin_hlc::HLC::new(0, 0), // Not applicable for branch copy
            node_id: None,
            source_branch: source_branch.clone(),
            created_at: chrono::Utc::now(),
        };

        self.job_store.as_ref().enqueue(&job)?;

        tracing::debug!(
            job_id = %job.job_id,
            branch = %branch_name,
            source_branch = ?source_branch,
            "Enqueued branch copy embedding job"
        );

        Ok(())
    }
}

impl<S: Storage> EventHandler for EmbeddingEventHandler<S> {
    fn name(&self) -> &str {
        "embedding_generator"
    }

    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            match event {
                Event::Node(node_event) => {
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
                            "Failed to enqueue embedding job"
                        );
                    }

                    result.map_err(|e| anyhow::anyhow!(e))
                }
                Event::Repository(repo_event) => {
                    if repo_event.kind == RepositoryEventKind::BranchCreated {
                        let result = self.handle_branch_created(repo_event).await;

                        if let Err(e) = &result {
                            tracing::error!(
                                error = %e,
                                repo_id = %repo_event.repository_id,
                                "Failed to enqueue branch copy embedding job"
                            );
                        }

                        result.map_err(|e| anyhow::anyhow!(e))
                    } else {
                        Ok(())
                    }
                }
                _ => Ok(()),
            }
        })
    }
}
