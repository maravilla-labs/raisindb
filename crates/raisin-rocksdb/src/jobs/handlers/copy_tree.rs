//! Copy tree execution job handler
//!
//! This module handles background copy tree execution for operations that affect
//! a large number of nodes (more than 5000). When a SQL COPY TREE command would
//! affect many nodes, the operation is executed asynchronously via this handler.
//!
//! # Architecture Note
//!
//! Due to dependency structure (raisin-sql-execution depends on raisin-rocksdb),
//! the actual copy operation is done by a callback provided at runtime rather than
//! directly using NodeService here. The transport layer (HTTP/WS) provides the
//! executor callback when starting the job system.

use raisin_error::{Error, Result};
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use std::sync::Arc;

/// Callback type for copy tree execution
///
/// This callback is provided by the transport layer which has access to NodeService.
/// It takes (source_id, target_parent_id, new_name, recursive, tenant_id, repo_id, branch, workspace, actor)
/// and returns the number of nodes copied.
pub type CopyTreeExecutorCallback = Arc<
    dyn Fn(
            String,
            String,
            Option<String>,
            bool,
            String,
            String,
            String,
            String,
            String,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<i64>> + Send>>
        + Send
        + Sync,
>;

/// Handler for copy tree execution jobs
///
/// This handler processes CopyTree jobs. Due to the dependency structure,
/// the actual copy operation is delegated to a callback provided by the
/// transport layer which has access to NodeService.
pub struct CopyTreeHandler {
    /// Optional copy tree executor callback (set by transport layer)
    executor: Option<CopyTreeExecutorCallback>,
}

impl CopyTreeHandler {
    /// Create a new copy tree job handler
    pub fn new() -> Self {
        Self { executor: None }
    }

    /// Set the copy tree executor callback
    ///
    /// This should be called by the transport layer after initialization
    /// to provide the NodeService-based executor.
    pub fn with_executor(mut self, executor: CopyTreeExecutorCallback) -> Self {
        self.executor = Some(executor);
        self
    }

    /// Handle copy tree execution job
    ///
    /// If no executor is configured, returns an error indicating that
    /// copy tree execution is not available.
    ///
    /// # Arguments
    ///
    /// * `job` - Job information containing the JobType::CopyTree variant
    /// * `context` - Job context with tenant, repo, branch, workspace info
    pub async fn handle(&self, job: &JobInfo, context: &JobContext) -> Result<()> {
        // Extract parameters from JobType
        let (source_id, target_parent_id, new_name, recursive) = match &job.job_type {
            JobType::CopyTree {
                source_id,
                target_parent_id,
                new_name,
                recursive,
            } => (
                source_id.clone(),
                target_parent_id.clone(),
                new_name.clone(),
                *recursive,
            ),
            _ => return Err(Error::Validation("Expected CopyTree job type".to_string())),
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
            source_id = %source_id,
            target_parent_id = %target_parent_id,
            new_name = ?new_name,
            recursive = recursive,
            actor = %actor,
            "Processing copy tree execution job"
        );

        // Check if executor is available
        let executor = self.executor.as_ref().ok_or_else(|| {
            Error::Validation(
                "Copy tree executor not configured. The transport layer must provide the executor callback.".to_string()
            )
        })?;

        // Execute via callback
        let start = std::time::Instant::now();
        let nodes_copied = executor(
            source_id.clone(),
            target_parent_id.clone(),
            new_name.clone(),
            recursive,
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
            source_id = %source_id,
            target_parent_id = %target_parent_id,
            nodes_copied = nodes_copied,
            elapsed_ms = elapsed.as_millis(),
            "Copy tree execution completed"
        );

        Ok(())
    }
}

impl Default for CopyTreeHandler {
    fn default() -> Self {
        Self::new()
    }
}
