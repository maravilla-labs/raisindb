//! Replication event handling for the event handler
//!
//! Handles replication-specific events such as operation batch application,
//! which may trigger lazy property index builds.

use super::UnifiedJobEventHandler;
use raisin_error::Result;
use raisin_events::ReplicationEvent;
use raisin_storage::jobs::{JobContext, JobType};
use std::collections::HashMap;

impl UnifiedJobEventHandler {
    /// Handle operation batch applied events (for lazy indexing trigger)
    pub(crate) async fn handle_operation_batch_applied(
        &self,
        repl_event: &ReplicationEvent,
    ) -> Result<()> {
        tracing::debug!(
            tenant_id = %repl_event.tenant_id,
            repo_id = %repl_event.repository_id,
            operation_count = repl_event.operation_count,
            "Processing OperationBatchApplied event"
        );

        // For catch-up scenarios (large operation batches), check if property indexes
        // need to be built for the common branch/workspace combinations.
        let branch = "main";
        let workspace = "default";

        // Check if property index has been built for this scope
        let index_status = match self.storage.lazy_index_manager().get_property_index_status(
            &repl_event.tenant_id,
            &repl_event.repository_id,
            branch,
            workspace,
        ) {
            Ok(status) => status,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    tenant_id = %repl_event.tenant_id,
                    repo_id = %repl_event.repository_id,
                    branch = %branch,
                    workspace = %workspace,
                    "Failed to check property index status"
                );
                return Ok(()); // Don't fail event processing
            }
        };

        if index_status.is_some() {
            tracing::debug!(
                tenant_id = %repl_event.tenant_id,
                repo_id = %repl_event.repository_id,
                branch = %branch,
                workspace = %workspace,
                "Property index already exists, skipping"
            );
            return Ok(());
        }

        // Property index doesn't exist - queue a build job
        tracing::info!(
            tenant_id = %repl_event.tenant_id,
            repo_id = %repl_event.repository_id,
            branch = %branch,
            workspace = %workspace,
            "Property index not found after replication catch-up, queuing build job"
        );

        let context = JobContext {
            tenant_id: repl_event.tenant_id.clone(),
            repo_id: repl_event.repository_id.clone(),
            branch: branch.to_string(),
            workspace_id: workspace.to_string(),
            revision: raisin_hlc::HLC::new(0, 0), // Not applicable for index build
            metadata: HashMap::new(),
        };

        let result = self
            .enqueue_job(
                JobType::PropertyIndexBuild {
                    tenant_id: repl_event.tenant_id.clone(),
                    repo_id: repl_event.repository_id.clone(),
                    branch: branch.to_string(),
                    workspace: workspace.to_string(),
                },
                &context,
            )
            .await;

        if let Err(e) = &result {
            tracing::error!(
                error = %e,
                tenant_id = %repl_event.tenant_id,
                repo_id = %repl_event.repository_id,
                branch = %branch,
                workspace = %workspace,
                "Failed to enqueue property index build job"
            );
        } else {
            tracing::info!(
                tenant_id = %repl_event.tenant_id,
                repo_id = %repl_event.repository_id,
                branch = %branch,
                workspace = %workspace,
                "Successfully queued PropertyIndexBuild job"
            );
        }

        Ok(())
    }
}
