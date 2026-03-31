//! Restore tree execution job handler
//!
//! This module handles background restore tree execution for RESTORE TREE NODE
//! operations. When a SQL RESTORE TREE NODE command is executed, the operation
//! is processed asynchronously via this handler.
//!
//! # Architecture Note
//!
//! Due to dependency structure (raisin-sql-execution depends on raisin-rocksdb),
//! the actual restore operation is done by a callback provided at runtime rather than
//! directly using NodeService here. The transport layer (HTTP/WS) provides the
//! executor callback when starting the job system.

use raisin_error::{Error, Result};
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use std::sync::Arc;

/// Callback type for restore tree execution
///
/// This callback is provided by the transport layer which has access to NodeService.
/// It takes (node_id, node_path, revision_hlc, recursive, translations, tenant_id, repo_id, branch, workspace, actor)
/// and returns the number of nodes restored.
pub type RestoreTreeExecutorCallback = Arc<
    dyn Fn(
            String,              // node_id
            String,              // node_path
            String,              // revision_hlc
            bool,                // recursive
            Option<Vec<String>>, // translations
            String,              // tenant_id
            String,              // repo_id
            String,              // branch
            String,              // workspace
            String,              // actor
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<i64>> + Send>>
        + Send
        + Sync,
>;

/// Handler for restore tree execution jobs
///
/// This handler processes RestoreTree jobs. Due to the dependency structure,
/// the actual restore operation is delegated to a callback provided by the
/// transport layer which has access to NodeService.
pub struct RestoreTreeHandler {
    /// Optional restore tree executor callback (set by transport layer)
    executor: Option<RestoreTreeExecutorCallback>,
}

impl RestoreTreeHandler {
    /// Create a new restore tree job handler
    pub fn new() -> Self {
        Self { executor: None }
    }

    /// Set the restore tree executor callback
    ///
    /// This should be called by the transport layer after initialization
    /// to provide the NodeService-based executor.
    pub fn with_executor(mut self, executor: RestoreTreeExecutorCallback) -> Self {
        self.executor = Some(executor);
        self
    }

    /// Handle restore tree execution job
    ///
    /// If no executor is configured, returns an error indicating that
    /// restore tree execution is not available.
    ///
    /// # Arguments
    ///
    /// * `job` - Job information containing the JobType::RestoreTree variant
    /// * `context` - Job context with tenant, repo, branch, workspace info
    pub async fn handle(&self, job: &JobInfo, context: &JobContext) -> Result<()> {
        // Extract parameters from JobType
        let (node_id, node_path, revision_hlc, recursive, translations) = match &job.job_type {
            JobType::RestoreTree {
                node_id,
                node_path,
                revision_hlc,
                recursive,
                translations,
            } => (
                node_id.clone(),
                node_path.clone(),
                revision_hlc.clone(),
                *recursive,
                translations.clone(),
            ),
            _ => {
                return Err(Error::Validation(
                    "Expected RestoreTree job type".to_string(),
                ))
            }
        };

        // Extract actor from context metadata
        let actor = context
            .metadata
            .get("actor")
            .and_then(|v| v.as_str())
            .unwrap_or("system")
            .to_string();

        tracing::info!(
            job_id = %job.id,
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            branch = %context.branch,
            workspace_id = %context.workspace_id,
            node_id = %node_id,
            node_path = %node_path,
            revision_hlc = %revision_hlc,
            recursive = recursive,
            translations = ?translations,
            actor = %actor,
            "Processing restore tree execution job"
        );

        // Check if executor is available
        let executor = self.executor.as_ref().ok_or_else(|| {
            Error::Validation(
                "Restore tree executor not configured. The transport layer must provide the executor callback.".to_string()
            )
        })?;

        // Execute via callback
        let start = std::time::Instant::now();
        let nodes_restored = executor(
            node_id.clone(),
            node_path.clone(),
            revision_hlc.clone(),
            recursive,
            translations,
            context.tenant_id.clone(),
            context.repo_id.clone(),
            context.branch.clone(),
            context.workspace_id.clone(),
            actor,
        )
        .await?;

        let elapsed = start.elapsed();

        tracing::info!(
            job_id = %job.id,
            node_id = %node_id,
            node_path = %node_path,
            revision_hlc = %revision_hlc,
            nodes_restored = nodes_restored,
            elapsed_ms = elapsed.as_millis(),
            "Restore tree execution completed"
        );

        Ok(())
    }
}

impl Default for RestoreTreeHandler {
    fn default() -> Self {
        Self::new()
    }
}
