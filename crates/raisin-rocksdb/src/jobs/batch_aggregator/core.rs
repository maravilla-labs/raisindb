//! Core batch aggregator implementation.
//!
//! Contains `BatchIndexAggregator` struct and all its methods for queuing,
//! flushing, and managing batch fulltext index operations.

use crate::jobs::{dispatcher::JobDispatcher, IndexKey, JobDataStore};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_storage::jobs::{
    BatchIndexOperation, IndexOperation, JobContext, JobId, JobRegistry, JobType,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

/// Configuration for batch aggregation behavior
#[derive(Debug, Clone)]
pub struct BatchAggregatorConfig {
    /// Maximum number of operations per batch before auto-flush
    pub max_batch_size: usize,
    /// Maximum time to hold operations before flushing (even if batch not full)
    pub flush_interval: Duration,
    /// Minimum batch size to trigger time-based flush
    /// (avoid flushing tiny batches during normal operation)
    pub min_flush_size: usize,
}

impl Default for BatchAggregatorConfig {
    fn default() -> Self {
        Self {
            max_batch_size: 1000,
            flush_interval: Duration::from_secs(300), // 5 minutes
            min_flush_size: 100,
        }
    }
}

/// Pending operation awaiting batch processing
struct PendingOperation {
    node_id: String,
    operation: IndexOperation,
    context: JobContext,
    queued_at: Instant,
}

/// Aggregates fulltext index operations into batch jobs
///
/// This service collects individual indexing operations and groups them
/// into batch jobs for more efficient processing. It dramatically improves
/// bulk import performance by reducing Tantivy commit overhead.
///
/// # Thread Safety
///
/// The aggregator is thread-safe and can be shared across multiple event handlers.
/// Operations are protected by an async RwLock.
pub struct BatchIndexAggregator {
    /// Pending operations grouped by index key (tenant/repo/branch)
    pending: Arc<RwLock<HashMap<IndexKey, Vec<PendingOperation>>>>,
    /// Configuration
    config: BatchAggregatorConfig,
    /// Job registry for creating batch jobs
    job_registry: Arc<JobRegistry>,
    /// Job data store for contexts
    job_data_store: Arc<JobDataStore>,
    /// Job dispatcher for routing jobs to worker queues
    dispatcher: Arc<JobDispatcher>,
}

impl BatchIndexAggregator {
    /// Create a new batch aggregator
    pub fn new(
        config: BatchAggregatorConfig,
        job_registry: Arc<JobRegistry>,
        job_data_store: Arc<JobDataStore>,
        dispatcher: Arc<JobDispatcher>,
    ) -> Self {
        tracing::info!(
            max_batch_size = config.max_batch_size,
            flush_interval_secs = config.flush_interval.as_secs(),
            min_flush_size = config.min_flush_size,
            "BatchIndexAggregator initialized"
        );

        Self {
            pending: Arc::new(RwLock::new(HashMap::new())),
            config,
            job_registry,
            job_data_store,
            dispatcher,
        }
    }

    /// Queue an operation for batch processing
    ///
    /// If the batch size threshold is reached, automatically flushes the batch.
    pub async fn queue(
        &self,
        node_id: &str,
        operation: IndexOperation,
        context: &JobContext,
    ) -> Result<()> {
        let key = IndexKey::new(&context.tenant_id, &context.repo_id, &context.branch);

        let should_flush = {
            let mut pending = self.pending.write().await;
            let ops = pending.entry(key.clone()).or_insert_with(Vec::new);

            ops.push(PendingOperation {
                node_id: node_id.to_string(),
                operation,
                context: context.clone(),
                queued_at: Instant::now(),
            });

            tracing::trace!(
                node_id = %node_id,
                tenant_id = %context.tenant_id,
                repo_id = %context.repo_id,
                branch = %context.branch,
                pending_count = ops.len(),
                "Queued operation for batch processing"
            );

            ops.len() >= self.config.max_batch_size
        };

        if should_flush {
            tracing::debug!(
                tenant_id = %context.tenant_id,
                repo_id = %context.repo_id,
                branch = %context.branch,
                "Batch size threshold reached, flushing"
            );
            self.flush(&key).await?;
        }

        Ok(())
    }

    /// Flush pending operations for an index key into a batch job
    pub async fn flush(&self, key: &IndexKey) -> Result<Option<JobId>> {
        let operations = {
            let mut pending = self.pending.write().await;
            pending.remove(key).unwrap_or_default()
        };

        if operations.is_empty() {
            return Ok(None);
        }

        // Use first operation's context as base (they should all be same tenant/repo/branch)
        let base_context = operations[0].context.clone();

        // Find the latest revision among all operations
        let max_revision = operations
            .iter()
            .map(|op| op.context.revision)
            .max()
            .unwrap_or(HLC::new(0, 0));

        // Build batch operations
        let batch_ops: Vec<BatchIndexOperation> = operations
            .into_iter()
            .map(|op| BatchIndexOperation {
                node_id: op.node_id,
                operation: op.operation,
            })
            .collect();

        let batch_size = batch_ops.len();

        // Store batch operations in context metadata
        let mut context = base_context;
        context.revision = max_revision; // Use latest revision for node lookups
        context.metadata.insert(
            "batch_operations".to_string(),
            serde_json::to_value(&batch_ops)
                .map_err(|e| raisin_error::Error::storage(e.to_string()))?,
        );

        // Create batch job using unified job system (JobRegistry + JobDataStore)
        let job_type = JobType::FulltextBatchIndex {
            operation_count: batch_size,
        };

        let job_id = self
            .job_registry
            .register_job(
                job_type.clone(),
                Some(context.tenant_id.clone()),
                None,
                None,
                None,
            )
            .await?;

        self.job_data_store.put(&job_id, &context)?;

        // Dispatch to priority queue
        let priority = job_type.default_priority();
        self.dispatcher.dispatch(job_id.clone(), priority).await;

        tracing::info!(
            job_id = %job_id,
            batch_size = batch_size,
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            branch = %context.branch,
            priority = %priority,
            "Created and dispatched fulltext batch index job"
        );

        Ok(Some(job_id))
    }

    /// Flush all pending batches that exceed the time threshold
    ///
    /// This is called periodically by the background flush task.
    pub async fn flush_expired(&self) -> Result<Vec<JobId>> {
        let keys_to_flush: Vec<IndexKey> = {
            let pending = self.pending.read().await;
            pending
                .iter()
                .filter(|(_, ops)| {
                    if ops.is_empty() {
                        return false;
                    }

                    // Check if batch is large enough or old enough
                    let is_large_enough = ops.len() >= self.config.min_flush_size;
                    let is_old_enough = ops[0].queued_at.elapsed() >= self.config.flush_interval;

                    is_large_enough && is_old_enough
                })
                .map(|(k, _)| k.clone())
                .collect()
        };

        let mut job_ids = Vec::new();
        for key in keys_to_flush {
            if let Some(job_id) = self.flush(&key).await? {
                job_ids.push(job_id);
            }
        }

        if !job_ids.is_empty() {
            tracing::debug!(jobs_created = job_ids.len(), "Flushed expired batches");
        }

        Ok(job_ids)
    }

    /// Background task that periodically flushes expired batches
    ///
    /// This task runs until the shutdown token is cancelled, checking for
    /// expired batches at regular intervals.
    pub async fn run_flush_task(self: Arc<Self>, shutdown: CancellationToken) {
        // Check twice per flush interval for responsive batching
        let check_interval = self.config.flush_interval / 2;

        tracing::info!(
            check_interval_ms = check_interval.as_millis(),
            "Batch aggregator flush task started"
        );

        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    tracing::info!("Batch aggregator flush task shutting down, flushing pending batches");
                    if let Err(e) = self.flush_all().await {
                        tracing::error!(error = %e, "Failed to flush pending batches on shutdown");
                    }
                    break;
                }
                _ = tokio::time::sleep(check_interval) => {
                    if let Err(e) = self.flush_expired().await {
                        tracing::error!(error = %e, "Failed to flush expired batches");
                    }
                }
            }
        }

        tracing::info!("Batch aggregator flush task stopped");
    }

    /// Flush all pending operations regardless of thresholds
    ///
    /// Used during graceful shutdown to ensure no operations are lost.
    pub async fn flush_all(&self) -> Result<()> {
        let keys: Vec<IndexKey> = {
            let pending = self.pending.read().await;
            pending.keys().cloned().collect()
        };

        let mut flushed_count = 0;
        for key in keys {
            if let Some(_job_id) = self.flush(&key).await? {
                flushed_count += 1;
            }
        }

        if flushed_count > 0 {
            tracing::info!(
                batches_flushed = flushed_count,
                "Flushed all pending batches"
            );
        }

        Ok(())
    }

    /// Get current pending operation counts by index key
    ///
    /// Useful for monitoring and debugging.
    pub async fn pending_counts(&self) -> HashMap<String, usize> {
        let pending = self.pending.read().await;
        pending
            .iter()
            .map(|(k, v)| {
                (
                    format!("{}/{}/{}", k.tenant_id, k.repo_id, k.branch_name),
                    v.len(),
                )
            })
            .collect()
    }
}
