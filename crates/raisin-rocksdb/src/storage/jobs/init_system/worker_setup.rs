//! Worker pool and background task setup
//!
//! Creates the three-pool worker system, batch aggregator, event handler,
//! and starts background maintenance tasks (watchdog, cleanup).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::config::{JobPoolConfig, JobPoolsConfig};
use crate::jobs::JobDataStore;
use crate::jobs::{
    dispatcher::{JobDispatcher, JobReceiver},
    AIToolCallExecutionHandler, BatchAggregatorConfig, BatchIndexAggregator, JobHandlerRegistry,
    RocksDBWorkerPool, TriggerRegistry, UnifiedJobEventHandler,
};
use crate::storage::RocksDBStorage;
use raisin_storage::jobs::{JobCategory, JobRegistry, JobType};

/// Create the multi-pool worker system with per-category isolation
///
/// Each category (Realtime, Background, System) gets its own pool with a
/// dedicated runtime, receiver, and concurrency limits.
pub fn create_multi_pool(
    storage: Arc<RocksDBStorage>,
    job_registry: Arc<JobRegistry>,
    job_data_store: Arc<JobDataStore>,
    handlers: Arc<JobHandlerRegistry>,
    dispatcher: Arc<JobDispatcher>,
    receivers: HashMap<JobCategory, JobReceiver>,
    runtimes: HashMap<JobCategory, tokio::runtime::Handle>,
    pools_config: &JobPoolsConfig,
) -> Arc<RocksDBWorkerPool> {
    let mut configs = HashMap::new();
    configs.insert(JobCategory::Realtime, pools_config.realtime.clone());
    configs.insert(JobCategory::Background, pools_config.background.clone());
    configs.insert(JobCategory::System, pools_config.system.clone());

    Arc::new(RocksDBWorkerPool::new_multi_pool(
        storage,
        job_registry,
        job_data_store,
        handlers,
        dispatcher,
        receivers,
        runtimes,
        configs,
    ))
}

/// Create a single-pool worker system (backward compatibility / tests)
pub fn create_worker_pool(
    storage: Arc<RocksDBStorage>,
    job_registry: Arc<JobRegistry>,
    job_data_store: Arc<JobDataStore>,
    handlers: Arc<JobHandlerRegistry>,
    dispatcher: Arc<JobDispatcher>,
    receiver: JobReceiver,
    worker_runtime: tokio::runtime::Handle,
) -> Arc<RocksDBWorkerPool> {
    Arc::new(RocksDBWorkerPool::new(
        storage.config.worker_pool_size,
        storage,
        job_registry,
        job_data_store,
        handlers,
        dispatcher,
        receiver,
        worker_runtime,
    ))
}

/// Create and start the batch index aggregator for efficient bulk import indexing
pub fn start_batch_aggregator(
    job_registry: Arc<JobRegistry>,
    job_data_store: Arc<JobDataStore>,
    dispatcher: Arc<JobDispatcher>,
) -> (Arc<BatchIndexAggregator>, CancellationToken) {
    let batch_aggregator = Arc::new(BatchIndexAggregator::new(
        BatchAggregatorConfig::default(),
        job_registry,
        job_data_store,
        dispatcher,
    ));

    let shutdown = CancellationToken::new();
    let aggregator_for_task = batch_aggregator.clone();
    let shutdown_for_task = shutdown.clone();
    tokio::spawn(async move {
        aggregator_for_task.run_flush_task(shutdown_for_task).await;
    });

    (batch_aggregator, shutdown)
}

/// Subscribe the unified event handler to the event bus
pub fn subscribe_event_handler(
    storage: Arc<RocksDBStorage>,
    job_registry: Arc<JobRegistry>,
    job_data_store: Arc<JobDataStore>,
    dispatcher: Arc<JobDispatcher>,
    batch_aggregator: Arc<BatchIndexAggregator>,
) {
    let trigger_registry = Arc::new(TriggerRegistry::new(
        storage.clone(),
        Duration::from_secs(300), // 5 minute TTL
    ));

    let event_handler = Arc::new(
        UnifiedJobEventHandler::new(
            storage.clone(),
            job_registry,
            job_data_store,
            dispatcher,
            storage.processing_rules_repository(),
        )
        .with_batch_aggregator(batch_aggregator)
        .with_trigger_registry(trigger_registry),
    );

    storage.event_bus.subscribe(event_handler);
}

/// Restore pending jobs and dispatch them to workers
pub async fn restore_and_dispatch_jobs(
    storage: &RocksDBStorage,
    dispatcher: &Arc<JobDispatcher>,
) -> raisin_error::Result<crate::storage::RestoreStats> {
    let restore_stats = storage.restore_pending_jobs().await?;

    let restored_jobs = storage.job_registry.list_jobs().await;
    let mut dispatched_count = 0;
    for job in restored_jobs {
        if matches!(job.status, raisin_storage::jobs::JobStatus::Scheduled) {
            let priority = job.job_type.default_priority();
            let category = job.job_type.category();
            dispatcher
                .dispatch_categorized(job.id.clone(), priority, category)
                .await;
            dispatched_count += 1;
        }
    }
    if dispatched_count > 0 {
        tracing::info!(
            dispatched_count = dispatched_count,
            "Dispatched restored jobs to worker queues"
        );
    }

    Ok(restore_stats)
}

/// Start background maintenance tasks (timeout watchdog, job cleanup)
pub fn start_background_tasks(
    storage: &RocksDBStorage,
    ai_tool_call_handler: Arc<AIToolCallExecutionHandler<RocksDBStorage>>,
) {
    let job_data_store = storage.job_data_store.clone();
    let timeout_callback: crate::jobs::OnJobTimeoutFn = Arc::new(move |job, error_msg| {
        let job_data_store = job_data_store.clone();
        let ai_tool_call_handler = ai_tool_call_handler.clone();
        Box::pin(async move {
            if !matches!(job.job_type, JobType::AIToolCallExecution { .. }) {
                return;
            }

            let context = match job_data_store.get(&job.id) {
                Ok(Some(ctx)) => ctx,
                Ok(None) => {
                    tracing::warn!(
                        job_id = %job.id,
                        "Timeout recovery skipped: missing job context"
                    );
                    return;
                }
                Err(e) => {
                    tracing::warn!(
                        job_id = %job.id,
                        error = %e,
                        "Timeout recovery skipped: failed to load job context"
                    );
                    return;
                }
            };

            if let Err(e) = ai_tool_call_handler
                .recover_timeout(&job, &context, &error_msg)
                .await
            {
                tracing::error!(
                    job_id = %job.id,
                    error = %e,
                    "AIToolCall timeout recovery failed"
                );
            }
        })
    });

    // Start timeout watchdog
    let watchdog_shutdown = CancellationToken::new();
    let watchdog =
        crate::jobs::TimeoutWatchdog::new(storage.job_registry.clone(), watchdog_shutdown.clone())
            .with_timeout_callback(timeout_callback);
    tokio::spawn(async move {
        watchdog.run().await;
    });

    // Start cleanup task (24 hour retention)
    let cleanup_shutdown = CancellationToken::new();
    let cleanup = crate::jobs::JobCleanupTask::new(
        storage.job_metadata_store.clone(),
        24, // retention hours
        cleanup_shutdown.clone(),
    );
    tokio::spawn(async move {
        cleanup.run().await;
    });
}
