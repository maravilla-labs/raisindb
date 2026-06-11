//! Core batch aggregator implementation.
//!
//! Contains `BatchIndexAggregator` struct and all its methods for queuing,
//! flushing, and managing batch fulltext index operations.
//!
//! # Flush gates
//!
//! Three independent OR-conditions trigger a flush for a key:
//!
//! 1. `ops.len() >= max_batch_size` — bulk-import cap.
//! 2. `ops.len() >= min_flush_size && oldest.elapsed() >= flush_interval`
//!    — bulk amortization window. Each new `queue()` resets the
//!    aggregator's view of "oldest", so this clause never trips
//!    mid-burst.
//! 3. `now - last_push >= idle_flush_interval` — interactive UX:
//!    single-op edits flush in ~2 s even with no following traffic.
//!
//! # Durability
//!
//! When constructed via `with_persistence(db)`, every `queue()`
//! mirrors the op into the `pending_batch_ops` column family under
//! the in-memory write lock. `flush()` deletes the persisted records
//! only **after** the dispatched job is durable in the job system
//! (registered + context stored), so a crash anywhere in the path
//! either replays the op or runs it via the restored job — never
//! drops it.

use crate::jobs::{dispatcher::JobDispatcher, IndexKey, JobDataStore};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_storage::jobs::{
    BatchIndexOperation, IndexOperation, JobContext, JobId, JobRegistry, JobType,
};
use rocksdb::DB;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use super::persistence::{
    make_pending_key, now_nanos, PendingOpKey, PendingOpRecord, PendingPersistence,
};

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
    /// Per-key debounce window: once a key sees no new `queue()`
    /// activity for this long, its pending ops flush regardless of
    /// size. Single-op interactive edits ship in ~`idle_flush_interval`.
    /// Bulk-import bursts naturally re-batch because each push resets
    /// the per-key idle timer.
    pub idle_flush_interval: Duration,
}

impl Default for BatchAggregatorConfig {
    fn default() -> Self {
        Self {
            max_batch_size: 1000,
            flush_interval: Duration::from_secs(300), // 5 minutes
            min_flush_size: 100,
            idle_flush_interval: Duration::from_secs(2),
        }
    }
}

/// Pending operation awaiting batch processing
struct PendingOperation {
    node_id: String,
    operation: IndexOperation,
    context: JobContext,
    queued_at: Instant,
    /// Key under which this op is stored in the `pending_batch_ops`
    /// CF. Empty when persistence is disabled (test paths). Used by
    /// `flush()` to remove the record once the job is durable.
    pending_key: PendingOpKey,
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
    /// Wall-clock of most recent `queue()` per key. Drives idle-flush.
    /// Held in a sibling map so the `pending` write-lock window stays
    /// narrow — no value-type widening on the hot path.
    last_push_at: Arc<RwLock<HashMap<IndexKey, Instant>>>,
    /// Configuration
    config: BatchAggregatorConfig,
    /// Job registry for creating batch jobs
    job_registry: Arc<JobRegistry>,
    /// Job data store for contexts
    job_data_store: Arc<JobDataStore>,
    /// Job dispatcher for routing jobs to worker queues
    dispatcher: Arc<JobDispatcher>,
    /// Optional durability backing. When `Some`, every `queue()`
    /// persists to RocksDB and survives process restart via
    /// `replay_pending`.
    persistence: Option<PendingPersistence>,
}

impl BatchIndexAggregator {
    /// Create a new batch aggregator without durability.
    ///
    /// Use [`with_persistence`](Self::with_persistence) on the result
    /// to opt into the RocksDB-backed crash-recovery buffer. Tests
    /// generally don't need it; production wiring in
    /// `worker_setup::start_batch_aggregator` always enables it.
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
            idle_flush_interval_ms = config.idle_flush_interval.as_millis(),
            "BatchIndexAggregator initialized"
        );

        Self {
            pending: Arc::new(RwLock::new(HashMap::new())),
            last_push_at: Arc::new(RwLock::new(HashMap::new())),
            config,
            job_registry,
            job_data_store,
            dispatcher,
            persistence: None,
        }
    }

    /// Enable RocksDB-backed durability for pending ops.
    ///
    /// Once enabled, single-op edits queued before the idle-flush
    /// window fires will survive SIGKILL / crash / host reboot:
    /// startup calls [`replay_pending`](Self::replay_pending) to
    /// refill the in-memory map from the `pending_batch_ops` CF.
    pub fn with_persistence(mut self, db: Arc<DB>) -> Self {
        self.persistence = Some(PendingPersistence::new(db));
        self
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

        // Persist BEFORE in-memory mutation so a crash between the two
        // is recoverable: we may have a stale CF row replayed, but
        // never an in-memory op without a CF row.
        let queued_at_nanos = now_nanos();
        let pending_key = if let Some(ref p) = self.persistence {
            let k = make_pending_key(
                &context.tenant_id,
                &context.repo_id,
                &context.branch,
                queued_at_nanos,
            );
            let record = PendingOpRecord {
                tenant_id: context.tenant_id.clone(),
                repo_id: context.repo_id.clone(),
                branch: context.branch.clone(),
                node_id: node_id.to_string(),
                operation: operation.clone(),
                context: context.clone(),
                queued_at_nanos,
            };
            p.put(&k, &record)?;
            k
        } else {
            Vec::new()
        };

        let now = Instant::now();

        let should_flush = {
            let mut pending = self.pending.write().await;
            let ops = pending.entry(key.clone()).or_insert_with(Vec::new);

            ops.push(PendingOperation {
                node_id: node_id.to_string(),
                operation,
                context: context.clone(),
                queued_at: now,
                pending_key,
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

        // Update last_push_at after releasing the pending lock to
        // keep both maps' write-lock windows independent and brief.
        {
            let mut last = self.last_push_at.write().await;
            last.insert(key.clone(), now);
        }

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
            // Even with no ops, drop the idle marker so the key stops
            // appearing in `flush_expired`'s candidate set.
            self.last_push_at.write().await.remove(key);
            return Ok(None);
        }

        // Collect pending-CF keys so we can delete them once the job
        // is durably registered. See module docs on the
        // commit-then-delete invariant.
        let pending_keys: Vec<PendingOpKey> = operations
            .iter()
            .filter(|op| !op.pending_key.is_empty())
            .map(|op| op.pending_key.clone())
            .collect();

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

        // Store the context BEFORE registering so dispatch can never
        // observe the job without its context.
        let job_id = raisin_storage::jobs::JobId::new();
        self.job_data_store.put(&job_id, &context)?;

        self.job_registry
            .register_job_with_id(
                job_id.clone(),
                job_type.clone(),
                context.tenant_id.clone(),
                None,
                None,
                None,
            )
            .await?;

        // At this point the job is durable: JobRegistry has the
        // metadata (Scheduled) and JobDataStore has the context with
        // batch_operations. A crash before dispatch will be picked up
        // by `restore_and_dispatch_jobs` on next start. Safe to clear
        // the pending-CF rows now.
        if let Some(ref p) = self.persistence {
            if !pending_keys.is_empty() {
                p.delete_batch(&pending_keys)?;
            }
        }

        // Drop the idle marker — fresh queues will re-create it.
        self.last_push_at.write().await.remove(key);

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

    /// Flush all pending batches whose flush gates are tripped.
    ///
    /// Three OR-conditions per key:
    /// - `ops.len() >= max_batch_size` (bulk-import cap)
    /// - `ops.len() >= min_flush_size && oldest.elapsed() >= flush_interval`
    /// - `last_push_at.elapsed() >= idle_flush_interval` (interactive UX)
    ///
    /// Read locks are held only long enough to compute the candidate
    /// key set; the per-key flushes happen outside the lock so
    /// dispatch / serde / I/O never block writers.
    pub async fn flush_expired(&self) -> Result<Vec<JobId>> {
        let now = Instant::now();
        let keys_to_flush: Vec<IndexKey> = {
            // Acquire both reads briefly to snapshot the predicate inputs.
            let pending = self.pending.read().await;
            let last_push = self.last_push_at.read().await;
            pending
                .iter()
                .filter(|(k, ops)| {
                    if ops.is_empty() {
                        return false;
                    }

                    let is_max = ops.len() >= self.config.max_batch_size;
                    let is_bulk_window = ops.len() >= self.config.min_flush_size
                        && ops[0].queued_at.elapsed() >= self.config.flush_interval;
                    let is_idle = last_push
                        .get(*k)
                        .map(|t| now.duration_since(*t) >= self.config.idle_flush_interval)
                        .unwrap_or(true);

                    is_max || is_bulk_window || is_idle
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
        // Poll twice per idle-flush window so single-op edits land
        // within ~`idle_flush_interval`. One brief read-lock per
        // tick — non-contentious in profiling.
        let check_interval = std::cmp::max(
            self.config.idle_flush_interval / 2,
            Duration::from_millis(100),
        );

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

    /// Replay persisted pending ops into the in-memory map.
    ///
    /// Called once at startup (after the aggregator and worker pool
    /// are constructed but before `restore_and_dispatch_jobs`) so any
    /// `queue()` that didn't complete its flush before the last
    /// process exit gets a fresh idle-flush window.
    ///
    /// If persistence is disabled this is a no-op.
    pub async fn replay_pending(self: &Arc<Self>) -> Result<usize> {
        let Some(ref p) = self.persistence else {
            return Ok(0);
        };
        let records = p.scan()?;
        if records.is_empty() {
            return Ok(0);
        }

        let count = records.len();
        let now = Instant::now();
        let mut pending = self.pending.write().await;
        let mut last_push = self.last_push_at.write().await;

        for (pending_key, record) in records {
            let key = IndexKey::new(&record.tenant_id, &record.repo_id, &record.branch);
            let ops = pending.entry(key.clone()).or_insert_with(Vec::new);
            ops.push(PendingOperation {
                node_id: record.node_id,
                operation: record.operation,
                context: record.context,
                // Start the bulk-window timer fresh — we deliberately
                // don't carry the old wall-clock over, since
                // `flush_interval` is measured against in-process
                // `Instant`. The idle-flush clause (~2 s) will fire
                // shortly regardless.
                queued_at: now,
                pending_key,
            });
            // Set last_push such that idle-flush triggers on the next
            // poll tick — no point waiting another 2 s after a
            // restart for ops that were already old.
            last_push.insert(key, now - self.config.idle_flush_interval);
        }

        tracing::info!(
            replayed_ops = count,
            "Restored pending batch ops from RocksDB"
        );

        Ok(count)
    }
}
