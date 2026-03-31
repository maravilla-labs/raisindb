//! Job enqueueing helpers for the event handler
//!
//! Provides methods for enqueuing jobs with deduplication support
//! and optional batch aggregation for fulltext indexing.

use super::UnifiedJobEventHandler;
use raisin_error::Result;
use raisin_storage::jobs::{IndexOperation, JobContext, JobId, JobType};

impl UnifiedJobEventHandler {
    /// Enqueue a job with its context (idempotent)
    ///
    /// Uses idempotent registration to prevent duplicate jobs when
    /// the same event is processed multiple times. The dedup key is
    /// generated from the JobContext (tenant/repo/branch/workspace)
    /// plus the job type's internal dedup key.
    ///
    /// IMPORTANT: Context is stored BEFORE job registration to avoid race
    /// conditions with DispatchingMonitor auto-dispatch. The job_id is
    /// pre-generated so context can be stored before the job is dispatched.
    pub(crate) async fn enqueue_job(&self, job_type: JobType, context: &JobContext) -> Result<()> {
        // Generate dedup key: context scope + job type specifics
        let dedup_key = format!(
            "{}:{}:{}:{}:{}",
            context.tenant_id,
            context.repo_id,
            context.branch,
            context.workspace_id,
            job_type.dedup_key()
        );

        // Pre-generate job_id so we can store context BEFORE registration
        // This avoids race condition where DispatchingMonitor dispatches
        // the job before context is available to workers
        let job_id = JobId::new();

        // Store context FIRST (before job is dispatched)
        self.job_data_store.put(&job_id, context)?;

        // Now register the job - this triggers DispatchingMonitor auto-dispatch
        let was_registered = self
            .job_registry
            .register_job_with_id_idempotent(
                job_id.clone(),
                job_type.clone(),
                Some(context.tenant_id.clone()),
                dedup_key.clone(),
                None,
            )
            .await?;

        if was_registered {
            tracing::debug!(
                job_id = %job_id,
                job_type = %job_type,
                tenant_id = %context.tenant_id,
                repo_id = %context.repo_id,
                "Enqueued job (context stored before dispatch)"
            );
        } else {
            // Job was skipped due to duplicate - clean up pre-stored context
            // This is fine because no worker will look for this job_id
            tracing::debug!(
                job_type = %job_type,
                dedup_key = %dedup_key,
                "Skipped duplicate job"
            );
        }

        Ok(())
    }

    /// Enqueue fulltext indexing job - uses batch aggregator if available
    ///
    /// When a batch aggregator is configured, operations are queued for batch
    /// processing. Otherwise, falls back to immediate single-node jobs.
    ///
    /// # Arguments
    ///
    /// * `node_id` - Node ID to index
    /// * `operation` - Index operation (add/update or delete)
    /// * `context` - Job context with tenant, repo, branch info
    pub(crate) async fn enqueue_fulltext_job(
        &self,
        node_id: &str,
        operation: IndexOperation,
        context: &JobContext,
    ) -> Result<()> {
        if let Some(aggregator) = &self.batch_aggregator {
            // Batch mode: queue for aggregation
            aggregator.queue(node_id, operation, context).await
        } else {
            // Legacy mode: immediate single-node job
            self.enqueue_job(
                JobType::FulltextIndex {
                    node_id: node_id.to_string(),
                    operation,
                },
                context,
            )
            .await
        }
    }
}
