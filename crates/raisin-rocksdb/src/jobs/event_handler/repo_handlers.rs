//! Repository-level event handling for the event handler
//!
//! Handles branch creation events by enqueuing fulltext and embedding
//! branch copy jobs.

use super::UnifiedJobEventHandler;
use raisin_error::Result;
use raisin_events::RepositoryEvent;
use raisin_storage::jobs::{JobContext, JobType};
use std::collections::HashMap;

impl UnifiedJobEventHandler {
    /// Handle branch creation events
    pub(crate) async fn handle_branch_created(&self, repo_event: &RepositoryEvent) -> Result<()> {
        let branch_name = repo_event.branch_name.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("BranchCreated event missing branch_name".to_string())
        })?;

        // Extract source branch from metadata (if provided)
        let source_branch = repo_event
            .metadata
            .as_ref()
            .and_then(|m| m.get("source_branch"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "main".to_string());

        let context = JobContext {
            tenant_id: repo_event.tenant_id.clone(),
            repo_id: repo_event.repository_id.clone(),
            branch: branch_name.clone(),
            workspace_id: repo_event
                .workspace
                .clone()
                .unwrap_or_else(|| "default".to_string()),
            revision: raisin_hlc::HLC::new(0, 0), // Not applicable for branch copy
            metadata: {
                let mut map = HashMap::new();
                map.insert(
                    "source_branch".to_string(),
                    serde_json::Value::String(source_branch.clone()),
                );
                map
            },
        };

        // Always enqueue fulltext branch copy job
        if let Err(e) = self
            .enqueue_job(
                JobType::FulltextBranchCopy {
                    source_branch: source_branch.clone(),
                },
                &context,
            )
            .await
        {
            tracing::error!(
                error = %e,
                branch = %branch_name,
                source_branch = %source_branch,
                "Failed to enqueue fulltext branch copy job"
            );
        }

        // Check if embeddings are enabled for this tenant
        if self.embeddings_enabled(&repo_event.tenant_id).await? {
            if let Err(e) = self
                .enqueue_job(
                    JobType::EmbeddingBranchCopy {
                        source_branch: source_branch.clone(),
                    },
                    &context,
                )
                .await
            {
                tracing::error!(
                    error = %e,
                    branch = %branch_name,
                    source_branch = %source_branch,
                    "Failed to enqueue embedding branch copy job"
                );
            }
        }

        Ok(())
    }
}
