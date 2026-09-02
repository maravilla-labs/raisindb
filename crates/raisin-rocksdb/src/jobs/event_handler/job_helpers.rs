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
        let mut dedup_key = format!(
            "{}:{}:{}:{}:{}",
            context.tenant_id,
            context.repo_id,
            context.branch,
            context.workspace_id,
            job_type.dedup_key()
        );

        // `EmbeddingGenerate` additionally keys on the REVISION, because it is
        // the one job here that reads the node at `context.revision` and whose
        // whole output is decided by what it reads.
        //
        // # The bug this fixes, reproduced live
        //
        // Uploading a PDF writes the asset node, which enqueues job A. The
        // extraction job then stores the extracted text on that node, which
        // emits `node:updated` and tries to enqueue job B. Both had the dedup
        // key `…:embedding_gen:{node_id}`, so B was DROPPED — and its context,
        // the only carrier of the newer revision, was discarded with it. Job A
        // then ran against the revision from BEFORE the text existed. The
        // document's body was extracted, stored, indexed for full text… and
        // never embedded, permanently, until something unrelated touched the
        // node again. Observed as `Skipped duplicate job
        // job_type=EmbeddingGenerate(...)` immediately after `Stored extraction
        // artifact on node`.
        //
        // # Why this is now cheap enough to do
        //
        // Dedup-by-node existed to stop a burst of edits calling the embedding
        // provider once per edit. That reason is gone: the spec hash decides
        // staleness from the actual INPUTS, and a run whose inputs have not
        // moved carries the previous revision's vectors forward instead of
        // calling the provider (`carry_forward_vectors`). So the extra runs a
        // burst now produces cost a row write each and no provider call — while
        // the collapse they replace cost a permanently missing embedding.
        //
        // # …and that cost argument holds only while the embedder is HEALTHY
        //
        // "A row write each and no provider call" assumes there is a prior good
        // vector to carry forward. During an upstream outage there is not: the
        // first run stores nothing, so every subsequent revision's run finds
        // nothing to carry, calls the provider, and becomes a full retry cycle
        // against a dead endpoint. The tradeoff is correct in the healthy case
        // and silently INVERTS in exactly the case that hurts — 2026-09-02, 257
        // re-opened assets, 2,512 attempts, 3 successes.
        //
        // The dedup key is deliberately NOT the fix for that (narrowing it
        // brings back the permanently missing embedding above). The upstream
        // circuit breaker is: with the breaker open, a fresh job PARKS before
        // doing any work instead of attempting, so N revisions of a node
        // produce N parked jobs and zero provider calls. See
        // `crate::jobs::circuit_breaker`.
        //
        // Only this job type is keyed on the revision. The others here are
        // either not revision-scoped or genuinely idempotent per node, and
        // widening the rule to them would multiply job volume for no
        // correctness gain.
        if matches!(job_type, JobType::EmbeddingGenerate { .. }) {
            dedup_key.push(':');
            dedup_key.push_str(&context.revision.to_string());
        }

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
                context.tenant_id.clone(),
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
