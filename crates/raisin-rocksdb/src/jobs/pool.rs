//! Worker pool implementation with three-pool category isolation
//!
//! The worker pool manages lightweight dispatcher workers across three isolated
//! category pools (Realtime, Background, System). Each pool has its own:
//! - Dedicated tokio runtime (thread isolation)
//! - Job receiver (category-specific priority queues)
//! - Handler semaphore (concurrency limit)
//! - In-flight tracker (graceful shutdown)
//!
//! This prevents cross-category starvation: background indexing can never
//! block realtime AI conversations.

use crate::config::JobPoolConfig;
use crate::jobs::{
    dispatcher::{DispatcherStats, JobDispatcher, JobReceiver},
    handlers::JobHandlerRegistry,
    worker::InFlightTracker,
    JobDataStore, RocksDBWorker,
};
use crate::RocksDBStorage;
use async_trait::async_trait;
use raisin_error::Result;
use raisin_storage::jobs::{JobCategory, JobRegistry, JobStatus, WorkerPool, WorkerPoolStats};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Default maximum concurrent handler tasks per pool (fallback)
const DEFAULT_MAX_CONCURRENT_HANDLERS: usize = 50;

/// A single category's pool of workers with dedicated runtime
struct CategoryPool {
    /// Category this pool handles
    category: JobCategory,
    /// Number of dispatcher workers
    num_workers: usize,
    /// Receiver for this category's priority queues
    receiver: JobReceiver,
    /// Dedicated runtime handle for this category
    runtime: tokio::runtime::Handle,
    /// Semaphore limiting concurrent handler tasks
    handler_semaphore: Arc<Semaphore>,
    /// Tracker for in-flight handler tasks
    in_flight: InFlightTracker,
    /// Maximum concurrent handlers (for stats)
    max_concurrent_handlers: usize,
    /// Worker task handles
    worker_handles: Mutex<Vec<JoinHandle<()>>>,
    /// Shutdown signal for this category's workers
    shutdown: CancellationToken,
}

impl CategoryPool {
    fn new(
        category: JobCategory,
        config: &JobPoolConfig,
        receiver: JobReceiver,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        Self {
            category,
            num_workers: config.dispatcher_workers,
            receiver,
            runtime,
            handler_semaphore: Arc::new(Semaphore::new(config.max_concurrent_handlers)),
            in_flight: InFlightTracker::new(),
            max_concurrent_handlers: config.max_concurrent_handlers,
            worker_handles: Mutex::new(Vec::new()),
            shutdown: CancellationToken::new(),
        }
    }

    /// Start workers for this category pool
    async fn start(
        &self,
        storage: Arc<RocksDBStorage>,
        job_registry: Arc<JobRegistry>,
        job_data_store: Arc<JobDataStore>,
        handlers: Arc<JobHandlerRegistry>,
    ) -> Result<()> {
        let mut handles = self.worker_handles.lock().await;

        tracing::info!(
            category = %self.category,
            num_workers = self.num_workers,
            max_concurrent_handlers = self.max_concurrent_handlers,
            "Starting category pool"
        );

        for i in 0..self.num_workers {
            let worker = RocksDBWorker::new(
                i,
                storage.clone(),
                job_registry.clone(),
                job_data_store.clone(),
                handlers.clone(),
                self.shutdown.clone(),
                self.receiver.clone(),
                self.handler_semaphore.clone(),
                self.in_flight.clone(),
            );

            let handle = self.runtime.spawn(async move {
                worker.run().await;
            });

            handles.push(handle);
        }

        tracing::info!(
            category = %self.category,
            num_workers = self.num_workers,
            "Category pool started"
        );

        Ok(())
    }

    /// Stop workers and wait for in-flight tasks
    async fn stop(&self) {
        tracing::info!(category = %self.category, "Stopping category pool");

        self.shutdown.cancel();

        // Wait for dispatcher workers to finish
        let mut handles = self.worker_handles.lock().await;
        let worker_timeout = tokio::time::Duration::from_secs(5);
        let shutdown_start = tokio::time::Instant::now();

        for (i, handle) in handles.drain(..).enumerate() {
            let remaining = worker_timeout.saturating_sub(shutdown_start.elapsed());
            match tokio::time::timeout(remaining, handle).await {
                Ok(Ok(_)) => {
                    tracing::debug!(
                        category = %self.category,
                        worker_id = i,
                        "Worker stopped gracefully"
                    );
                }
                Ok(Err(e)) => {
                    tracing::error!(
                        category = %self.category,
                        worker_id = i,
                        error = %e,
                        "Worker panicked during shutdown"
                    );
                }
                Err(_) => {
                    tracing::warn!(
                        category = %self.category,
                        worker_id = i,
                        "Worker did not stop within timeout"
                    );
                }
            }
        }

        // Wait for in-flight handler tasks
        let in_flight_count = self.in_flight.get();
        if in_flight_count > 0 {
            tracing::info!(
                category = %self.category,
                in_flight = in_flight_count,
                "Waiting for in-flight handler tasks"
            );

            let handler_timeout = tokio::time::Duration::from_secs(30);
            match tokio::time::timeout(handler_timeout, async {
                while self.in_flight.get() > 0 {
                    self.in_flight.zero_notify.notified().await;
                }
            })
            .await
            {
                Ok(_) => {
                    tracing::info!(
                        category = %self.category,
                        "All in-flight handler tasks completed"
                    );
                }
                Err(_) => {
                    tracing::warn!(
                        category = %self.category,
                        remaining = self.in_flight.get(),
                        "Timed out waiting for in-flight handler tasks"
                    );
                }
            }
        }

        tracing::info!(category = %self.category, "Category pool stopped");
    }

    /// Get total active workers
    async fn active_workers(&self) -> usize {
        self.worker_handles.lock().await.len()
    }
}

/// Pool of workers for processing background jobs with three-pool isolation
///
/// Wraps three `CategoryPool` instances (Realtime, Background, System), each
/// with its own dedicated runtime, handler semaphore, and in-flight tracker.
/// This prevents cross-category starvation entirely.
pub struct RocksDBWorkerPool {
    /// Per-category pools
    pools: HashMap<JobCategory, CategoryPool>,
    /// RocksDB storage instance
    storage: Arc<RocksDBStorage>,
    /// Shared job registry
    job_registry: Arc<JobRegistry>,
    /// Job context data store
    job_data_store: Arc<JobDataStore>,
    /// Handler registry (shared across all pools)
    handlers: Arc<JobHandlerRegistry>,
    /// Job dispatcher for routing jobs to category queues
    dispatcher: Arc<JobDispatcher>,
    /// Master shutdown signal
    shutdown: CancellationToken,
}

impl RocksDBWorkerPool {
    /// Create a new dispatcher and per-category receiver map
    pub fn create_dispatcher() -> (Arc<JobDispatcher>, HashMap<JobCategory, JobReceiver>) {
        let (dispatcher, receivers) = JobDispatcher::new();
        (Arc::new(dispatcher), receivers)
    }

    /// Create a new multi-pool worker system
    ///
    /// Each category gets its own pool with a dedicated runtime, receiver,
    /// and concurrency limits from the provided configs and receivers.
    pub fn new_multi_pool(
        storage: Arc<RocksDBStorage>,
        job_registry: Arc<JobRegistry>,
        job_data_store: Arc<JobDataStore>,
        handlers: Arc<JobHandlerRegistry>,
        dispatcher: Arc<JobDispatcher>,
        receivers: HashMap<JobCategory, JobReceiver>,
        runtimes: HashMap<JobCategory, tokio::runtime::Handle>,
        configs: HashMap<JobCategory, JobPoolConfig>,
    ) -> Self {
        let mut pools = HashMap::new();

        for category in [
            JobCategory::Realtime,
            JobCategory::Background,
            JobCategory::System,
        ] {
            let receiver = receivers
                .get(&category)
                .cloned()
                .unwrap_or_else(|| panic!("{} receiver must exist", category));
            let runtime = runtimes
                .get(&category)
                .cloned()
                .unwrap_or_else(|| panic!("{} runtime must exist", category));
            let config = configs
                .get(&category)
                .unwrap_or_else(|| panic!("{} config must exist", category));

            pools.insert(
                category,
                CategoryPool::new(category, config, receiver, runtime),
            );
        }

        Self {
            pools,
            storage,
            job_registry,
            job_data_store,
            handlers,
            dispatcher,
            shutdown: CancellationToken::new(),
        }
    }

    /// Create a single-pool worker system (backward compatibility)
    ///
    /// All categories are served by one pool using the Realtime receiver.
    pub fn new(
        num_workers: usize,
        storage: Arc<RocksDBStorage>,
        job_registry: Arc<JobRegistry>,
        job_data_store: Arc<JobDataStore>,
        handlers: Arc<JobHandlerRegistry>,
        dispatcher: Arc<JobDispatcher>,
        receiver: JobReceiver,
        worker_runtime: tokio::runtime::Handle,
    ) -> Self {
        let config = JobPoolConfig {
            dispatcher_workers: num_workers,
            runtime_threads: 0, // Not used in single-pool mode
            max_concurrent_handlers: DEFAULT_MAX_CONCURRENT_HANDLERS,
        };

        let mut pools = HashMap::new();
        pools.insert(
            JobCategory::Realtime,
            CategoryPool::new(JobCategory::Realtime, &config, receiver, worker_runtime),
        );

        Self {
            pools,
            storage,
            job_registry,
            job_data_store,
            handlers,
            dispatcher,
            shutdown: CancellationToken::new(),
        }
    }

    /// Get the job dispatcher for enqueueing jobs
    pub fn dispatcher(&self) -> Arc<JobDispatcher> {
        self.dispatcher.clone()
    }

    /// Get dispatcher statistics (queue lengths, dispatch counts)
    pub fn dispatcher_stats(&self) -> DispatcherStats {
        self.dispatcher.stats()
    }

    /// Get the total number of currently in-flight handler tasks (all pools)
    pub fn in_flight_count(&self) -> usize {
        self.pools.values().map(|p| p.in_flight.get()).sum()
    }

    /// Get the handler semaphore for a specific category (for stats)
    pub fn handler_semaphore(&self, category: JobCategory) -> Option<&Arc<Semaphore>> {
        self.pools.get(&category).map(|p| &p.handler_semaphore)
    }

    /// Get the number of pending jobs in the queue
    async fn count_pending_jobs(&self) -> usize {
        let jobs = self.job_registry.list_jobs().await;
        jobs.iter()
            .filter(|job| matches!(job.status, JobStatus::Scheduled))
            .count()
    }

    /// Recover pending jobs on startup
    ///
    /// Scans the registry for Scheduled, Running, and Executing jobs that
    /// weren't in the dispatcher (e.g., after a crash) and redispatches them.
    /// Running/Executing jobs are reset to Scheduled first (crash recovery).
    pub async fn recover_pending_jobs(&self) {
        let jobs = self.job_registry.list_jobs().await;
        let recoverable: Vec<_> = jobs
            .into_iter()
            .filter(|job| {
                matches!(
                    job.status,
                    JobStatus::Scheduled | JobStatus::Running | JobStatus::Executing
                )
            })
            .collect();

        if recoverable.is_empty() {
            tracing::debug!("No pending jobs to recover");
            return;
        }

        tracing::info!(
            count = recoverable.len(),
            "Recovering pending jobs to dispatcher"
        );

        for job in recoverable {
            // Reset Running/Executing → Scheduled (crash recovery)
            if matches!(job.status, JobStatus::Running | JobStatus::Executing) {
                if let Err(e) = self
                    .job_registry
                    .update_status(&job.id, JobStatus::Scheduled)
                    .await
                {
                    tracing::warn!(
                        job_id = %job.id,
                        error = %e,
                        "Failed to reset job status during recovery"
                    );
                    continue;
                }
            }

            let priority = job.job_type.default_priority();
            let category = job.job_type.category();
            self.dispatcher
                .dispatch_categorized(job.id, priority, category)
                .await;
        }

        tracing::info!("Job recovery complete");
    }

    /// Start the periodic queue depth and in-flight logger
    fn start_queue_logger(&self) {
        let dispatcher = self.dispatcher.clone();
        let shutdown = self.shutdown.clone();

        // Collect per-pool in-flight trackers and semaphores for logging
        let pool_stats: Vec<(JobCategory, InFlightTracker, Arc<Semaphore>, usize)> = self
            .pools
            .iter()
            .map(|(&cat, pool)| {
                (
                    cat,
                    pool.in_flight.clone(),
                    pool.handler_semaphore.clone(),
                    pool.max_concurrent_handlers,
                )
            })
            .collect();

        // Use the first available runtime for the logger task
        let runtime = self
            .pools
            .values()
            .next()
            .map(|p| p.runtime.clone())
            .expect("At least one pool must exist");

        runtime.spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            interval.tick().await; // Skip immediate first tick
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let stats = dispatcher.stats();
                        let total_pending = stats.high_queue_len + stats.normal_queue_len + stats.low_queue_len;

                        let total_active: usize = pool_stats.iter().map(|(_, tracker, _, _)| tracker.get()).sum();

                        // Per-category logging
                        for (cat, tracker, semaphore, max_handlers) in &pool_stats {
                            let active = tracker.get();
                            let available = semaphore.available_permits();

                            if let Some(cat_stats) = stats.category_stats.get(cat) {
                                let cat_pending = cat_stats.high_queue_len + cat_stats.normal_queue_len + cat_stats.low_queue_len;

                                // Warn if any queue exceeds 80% capacity
                                let high_pct = (cat_stats.high_queue_len as f64 / 10_000.0) * 100.0;
                                let normal_pct = (cat_stats.normal_queue_len as f64 / 50_000.0) * 100.0;
                                let low_pct = (cat_stats.low_queue_len as f64 / 100_000.0) * 100.0;

                                if high_pct > 80.0 || normal_pct > 80.0 || low_pct > 80.0 {
                                    tracing::warn!(
                                        category = %cat,
                                        high = cat_stats.high_queue_len,
                                        normal = cat_stats.normal_queue_len,
                                        low = cat_stats.low_queue_len,
                                        active_handlers = active,
                                        handler_permits = format!("{}/{}", available, max_handlers),
                                        "Queue depth WARNING - nearing capacity"
                                    );
                                } else if cat_pending > 0 || active > 0 {
                                    tracing::info!(
                                        category = %cat,
                                        high = cat_stats.high_queue_len,
                                        normal = cat_stats.normal_queue_len,
                                        low = cat_stats.low_queue_len,
                                        active_handlers = active,
                                        handler_permits = format!("{}/{}", available, max_handlers),
                                        total_dispatched = cat_stats.total_high_dispatched + cat_stats.total_normal_dispatched + cat_stats.total_low_dispatched,
                                        "Queue depth"
                                    );
                                }
                            }
                        }
                    }
                    _ = shutdown.cancelled() => {
                        tracing::debug!("Queue depth logger shutting down");
                        break;
                    }
                }
            }
        });
    }
}

#[async_trait]
impl WorkerPool for RocksDBWorkerPool {
    /// Start all category pools
    async fn start(&self) -> Result<Vec<JoinHandle<()>>> {
        // Recover any pending jobs before starting workers
        self.recover_pending_jobs().await;

        tracing::info!(
            num_pools = self.pools.len(),
            "Starting RocksDB worker pool with category isolation"
        );

        // Start each category pool
        for pool in self.pools.values() {
            pool.start(
                self.storage.clone(),
                self.job_registry.clone(),
                self.job_data_store.clone(),
                self.handlers.clone(),
            )
            .await?;
        }

        // Start periodic queue depth logger
        self.start_queue_logger();

        tracing::info!("All category pools started successfully");
        Ok(vec![])
    }

    /// Stop all category pools gracefully
    async fn stop(&self) {
        tracing::info!("Stopping RocksDB worker pool");

        // Signal master shutdown
        self.shutdown.cancel();

        // Close dispatcher channels (all categories)
        self.dispatcher.close();

        // Stop each category pool (waits for in-flight tasks)
        for pool in self.pools.values() {
            pool.stop().await;
        }

        tracing::info!("RocksDB worker pool stopped");
    }

    /// Get aggregated worker pool statistics with per-category breakdown
    async fn stats(&self) -> WorkerPoolStats {
        let mut active_workers = 0;
        for pool in self.pools.values() {
            active_workers += pool.active_workers().await;
        }

        let pending_jobs = self.count_pending_jobs().await;

        let jobs = self.job_registry.list_jobs().await;
        let completed_jobs = jobs
            .iter()
            .filter(|job| matches!(job.status, JobStatus::Completed))
            .count() as u64;
        let failed_jobs = jobs
            .iter()
            .filter(|job| matches!(job.status, JobStatus::Failed(_)))
            .count() as u64;

        let processing_times: Vec<f64> = jobs
            .iter()
            .filter_map(|job| {
                if let Some(completed_at) = job.completed_at {
                    let duration = completed_at
                        .signed_duration_since(job.started_at)
                        .to_std()
                        .ok()?;
                    Some(duration.as_millis() as f64)
                } else {
                    None
                }
            })
            .collect();

        let avg_processing_time_ms = if !processing_times.is_empty() {
            Some(processing_times.iter().sum::<f64>() / processing_times.len() as f64)
        } else {
            None
        };

        // Build per-category stats
        let dispatcher_stats = self.dispatcher.stats();
        let mut category_stats = Vec::new();

        for (&cat, pool) in &self.pools {
            let cat_queue = dispatcher_stats
                .category_stats
                .get(&cat)
                .cloned()
                .unwrap_or_default();

            category_stats.push(raisin_storage::jobs::CategoryPoolStats {
                category: cat.to_string(),
                active_handler_tasks: pool.in_flight.get(),
                handler_permits_available: pool.handler_semaphore.available_permits(),
                handler_permits_max: pool.max_concurrent_handlers,
                queue_depth_high: cat_queue.high_queue_len,
                queue_depth_normal: cat_queue.normal_queue_len,
                queue_depth_low: cat_queue.low_queue_len,
                dispatcher_workers: pool.num_workers,
            });
        }

        // Sort by category name for deterministic ordering
        category_stats.sort_by(|a, b| a.category.cmp(&b.category));

        WorkerPoolStats {
            active_workers,
            pending_jobs,
            completed_jobs,
            failed_jobs,
            avg_processing_time_ms,
            category_stats,
        }
    }
}
