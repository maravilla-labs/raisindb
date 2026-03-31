//! RocksDB worker implementation for processing background jobs
//!
//! Workers are lightweight dispatchers that receive jobs from priority queues,
//! claim them, load context, and spawn independent handler tasks. Workers do NOT
//! await handler completion — they immediately return to receive the next job.
//! This non-blocking design prevents long-running handlers (AI, functions) from
//! blocking the dispatch pipeline.

use crate::jobs::{dispatcher::JobReceiver, handlers::JobHandlerRegistry, JobDataStore};
use crate::RocksDBStorage;
use raisin_error::Result;
use raisin_storage::jobs::{JobId, JobInfo, JobRegistry, JobStatus};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Notify, Semaphore};
use tokio_util::sync::CancellationToken;

/// Extract clean error message from OpenAI-style JSON responses
///
/// OpenAI API returns errors in JSON format like:
/// ```json
/// {
///   "error": {
///     "message": "...",
///     "type": "...",
///     "code": "..."
///   }
/// }
/// ```
///
/// This function extracts just the message field for cleaner error display.
fn extract_clean_error_message(error: &raisin_error::Error) -> String {
    let error_str = format!("{}", error);

    // Try to parse the error message for OpenAI-style JSON errors
    // Pattern: "OpenAI API error {status}: {json_body}"
    if let Some(json_start) = error_str.find("{\n") {
        let json_part = &error_str[json_start..];

        // Try to parse as JSON
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_part) {
            if let Some(error_obj) = json.get("error") {
                if let Some(message) = error_obj.get("message") {
                    if let Some(msg_str) = message.as_str() {
                        // Reconstruct the error message with just the extracted part
                        let prefix = &error_str[..json_start];
                        return format!("{}{}", prefix.trim_end_matches(": "), msg_str);
                    }
                }
            }
        }
    }

    // Fall back to original error message if extraction fails
    error_str
}

/// Shared state for tracking in-flight handler tasks
#[derive(Clone)]
pub struct InFlightTracker {
    /// Number of currently executing handler tasks
    pub count: Arc<AtomicUsize>,
    /// Notified when in-flight count reaches zero
    pub zero_notify: Arc<Notify>,
}

impl InFlightTracker {
    pub fn new() -> Self {
        Self {
            count: Arc::new(AtomicUsize::new(0)),
            zero_notify: Arc::new(Notify::new()),
        }
    }

    /// Increment in-flight count
    pub fn increment(&self) {
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement in-flight count and notify if zero
    pub fn decrement(&self) {
        let prev = self.count.fetch_sub(1, Ordering::Relaxed);
        if prev == 1 {
            self.zero_notify.notify_waiters();
        }
    }

    /// Get current in-flight count
    pub fn get(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }
}

/// Worker that dispatches jobs from priority queues to handler tasks
///
/// Each worker runs in its own task, receiving jobs via channels from the
/// dispatcher. Unlike the old design, workers do NOT await handler execution.
/// Instead, they spawn handler tasks and immediately return to the receive loop.
/// This makes workers non-blocking dispatchers (~1ms per job).
pub struct RocksDBWorker {
    /// Unique worker identifier for logging
    id: usize,
    /// RocksDB storage instance (may be needed by handlers)
    storage: Arc<RocksDBStorage>,
    /// Shared job registry for tracking job status
    job_registry: Arc<JobRegistry>,
    /// Job context data store
    job_data_store: Arc<JobDataStore>,
    /// Handler registry for dispatching jobs
    handlers: Arc<JobHandlerRegistry>,
    /// Shutdown signal
    shutdown: CancellationToken,
    /// Receiver for jobs from the dispatcher (priority queues)
    receiver: JobReceiver,
    /// Semaphore limiting concurrent handler tasks
    handler_semaphore: Arc<Semaphore>,
    /// Tracker for in-flight handler tasks
    in_flight: InFlightTracker,
}

impl RocksDBWorker {
    /// Create a new worker
    pub fn new(
        id: usize,
        storage: Arc<RocksDBStorage>,
        job_registry: Arc<JobRegistry>,
        job_data_store: Arc<JobDataStore>,
        handlers: Arc<JobHandlerRegistry>,
        shutdown: CancellationToken,
        receiver: JobReceiver,
        handler_semaphore: Arc<Semaphore>,
        in_flight: InFlightTracker,
    ) -> Self {
        Self {
            id,
            storage,
            job_registry,
            job_data_store,
            handlers,
            shutdown,
            receiver,
            handler_semaphore,
            in_flight,
        }
    }

    /// Main worker loop - receives jobs from dispatcher and spawns handler tasks
    ///
    /// Workers are lightweight dispatchers. They claim jobs and spawn independent
    /// handler tasks, then immediately return to receive the next job.
    pub async fn run(self) {
        tracing::info!(worker_id = self.id, "Worker started");

        loop {
            tokio::select! {
                // Check for shutdown signal
                _ = self.shutdown.cancelled() => {
                    tracing::info!(worker_id = self.id, "Worker received shutdown signal");
                    break;
                }

                // Receive job from dispatcher (blocks until job available)
                job_id_opt = self.receiver.recv() => {
                    match job_id_opt {
                        Some(job_id) => {
                            // Try to claim the job and spawn handler task
                            match self.try_claim_and_spawn(job_id).await {
                                Ok(true) => {
                                    // Handler task spawned successfully
                                }
                                Ok(false) => {
                                    tracing::trace!(
                                        worker_id = self.id,
                                        "Job already claimed by another worker"
                                    );
                                }
                                Err(e) => {
                                    tracing::error!(
                                        worker_id = self.id,
                                        error = %e,
                                        "Error spawning handler task"
                                    );
                                }
                            }
                        }
                        None => {
                            tracing::info!(
                                worker_id = self.id,
                                "All dispatcher channels closed, shutting down"
                            );
                            break;
                        }
                    }
                }
            }
        }

        tracing::info!(worker_id = self.id, "Worker stopped");
    }

    /// Try to claim a job and spawn a handler task for it
    ///
    /// Returns `Ok(true)` if job was claimed and handler spawned, `Ok(false)` if
    /// job was already claimed by another worker (normal MPMC behavior).
    async fn try_claim_and_spawn(&self, job_id: JobId) -> Result<bool> {
        // Try to claim the job atomically (Scheduled → Running)
        match self.job_registry.try_claim_job(&job_id).await {
            Ok(true) => {
                // Successfully claimed - get fresh job info
                match self.job_registry.get_job_info(&job_id).await {
                    Ok(job) => {
                        tracing::debug!(
                            worker_id = self.id,
                            job_id = %job.id,
                            job_type = %job.job_type,
                            "Claimed job, loading context"
                        );

                        // Load job context before spawning
                        let context = match self.job_data_store.get(&job.id)? {
                            Some(ctx) => ctx,
                            None => {
                                let error_msg = format!("Job context not found for job {}", job.id);
                                tracing::error!(
                                    worker_id = self.id,
                                    job_id = %job.id,
                                    "Missing job context"
                                );
                                self.job_registry.mark_failed(&job.id, error_msg).await?;
                                return Ok(true); // Claimed but failed
                            }
                        };

                        // Spawn independent handler task — worker is free immediately
                        let job_registry = self.job_registry.clone();
                        let job_data_store = self.job_data_store.clone();
                        let handlers = self.handlers.clone();
                        let handler_semaphore = self.handler_semaphore.clone();
                        let in_flight = self.in_flight.clone();
                        let job_id_for_handle = job.id.clone();

                        let handle = tokio::spawn(execute_handler_task(
                            job,
                            context,
                            job_registry,
                            job_data_store,
                            handlers,
                            handler_semaphore,
                            in_flight,
                        ));

                        if let Err(e) = self
                            .job_registry
                            .set_handle(&job_id_for_handle, handle)
                            .await
                        {
                            tracing::warn!(
                                worker_id = self.id,
                                job_id = %job_id_for_handle,
                                error = %e,
                                "Failed to register async job handle"
                            );
                        }

                        Ok(true)
                    }
                    Err(e) => {
                        tracing::warn!(
                            worker_id = self.id,
                            job_id = %job_id,
                            error = %e,
                            "Job disappeared after claim"
                        );
                        Ok(false)
                    }
                }
            }
            Ok(false) => Ok(false),
            Err(e) => {
                tracing::trace!(
                    worker_id = self.id,
                    job_id = %job_id,
                    error = %e,
                    "Failed to claim job"
                );
                Ok(false)
            }
        }
    }

    /// Scan registry for scheduled jobs (recovery/fallback method)
    #[allow(dead_code)]
    pub async fn find_scheduled_jobs(&self) -> Vec<JobId> {
        let jobs = self.job_registry.list_jobs().await;
        jobs.into_iter()
            .filter(|job| matches!(job.status, JobStatus::Scheduled))
            .map(|job| job.id)
            .collect()
    }
}

/// Independent handler task that runs to completion
///
/// This function runs as a spawned async task, completely independent of the
/// worker that dispatched it. It:
/// 1. Acquires a handler semaphore permit (backpressure)
/// 2. Starts its own heartbeat
/// 3. Calls the handler
/// 4. Updates job status based on result
/// 5. Cleans up context data
async fn execute_handler_task(
    job: JobInfo,
    context: raisin_storage::jobs::JobContext,
    job_registry: Arc<JobRegistry>,
    job_data_store: Arc<JobDataStore>,
    handlers: Arc<JobHandlerRegistry>,
    handler_semaphore: Arc<Semaphore>,
    in_flight: InFlightTracker,
) {
    in_flight.increment();

    // Start heartbeat task immediately after claim so queued-for-permit jobs
    // still report liveness and are governed by timeout watchdog.
    let heartbeat_token = CancellationToken::new();
    let heartbeat_task = {
        let registry = job_registry.clone();
        let job_id = job.id.clone();
        let token = heartbeat_token.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = token.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_secs(10)) => {
                        if let Err(e) = registry.update_heartbeat(&job_id).await {
                            tracing::error!(
                                job_id = %job_id,
                                error = %e,
                                "Failed to update heartbeat"
                            );
                        }
                    }
                }
            }
        })
    };

    // Update initial heartbeat before waiting on semaphore.
    if let Err(e) = job_registry.update_heartbeat(&job.id).await {
        tracing::warn!(
            job_id = %job.id,
            error = %e,
            "Failed to set initial heartbeat"
        );
    }

    // Acquire handler semaphore permit (limits concurrent handlers per pool)
    let _permit = match handler_semaphore.acquire().await {
        Ok(permit) => permit,
        Err(_) => {
            // Semaphore closed — shutting down
            tracing::warn!(
                job_id = %job.id,
                "Handler semaphore closed, marking job for retry"
            );
            let _ = job_registry
                .schedule_retry(
                    &job.id,
                    "Handler semaphore closed during shutdown".to_string(),
                )
                .await;
            heartbeat_token.cancel();
            heartbeat_task.await.ok();
            job_registry.clear_handle(&job.id).await;
            in_flight.decrement();
            return;
        }
    };

    tracing::info!(
        job_id = %job.id,
        job_type = %job.job_type,
        retry_count = job.retry_count,
        "Handler task started"
    );

    // Mark as Executing after permit acquisition.
    if let Err(e) = job_registry.mark_executing(&job.id).await {
        tracing::warn!(
            job_id = %job.id,
            error = %e,
            "Failed to mark job as executing after permit acquisition"
        );
        heartbeat_token.cancel();
        heartbeat_task.await.ok();
        job_registry.clear_handle(&job.id).await;
        in_flight.decrement();
        return;
    }

    // Dispatch to handler
    let dispatch_result = handlers.dispatch(&job, &context).await;

    // Stop heartbeat before handling result
    heartbeat_token.cancel();
    heartbeat_task.await.ok();

    // Handle result
    match dispatch_result {
        Ok(result_opt) => {
            tracing::info!(
                job_id = %job.id,
                job_type = %job.job_type,
                has_result = result_opt.is_some(),
                "Job completed successfully"
            );

            // Store result if handler returned one
            if let Some(result) = result_opt {
                if let Err(e) = job_registry.set_result(&job.id, result).await {
                    tracing::warn!(
                        job_id = %job.id,
                        error = %e,
                        "Failed to store job result"
                    );
                }
            }

            // Mark as completed
            if let Err(e) = job_registry.mark_completed(&job.id).await {
                tracing::error!(
                    job_id = %job.id,
                    error = %e,
                    "Failed to mark job as completed"
                );
            }

            // Delete job context data (cleanup)
            if let Err(e) = job_data_store.delete(&job.id) {
                tracing::warn!(
                    job_id = %job.id,
                    error = %e,
                    "Failed to delete job context after completion"
                );
            }
        }
        Err(e) => {
            let error_msg = extract_clean_error_message(&e);

            if job.retry_count < job.max_retries {
                tracing::warn!(
                    job_id = %job.id,
                    job_type = %job.job_type,
                    retry_count = job.retry_count + 1,
                    max_retries = job.max_retries,
                    error = %error_msg,
                    "Job failed, scheduling retry"
                );
                let _ = job_registry.schedule_retry(&job.id, error_msg).await;
            } else {
                tracing::error!(
                    job_id = %job.id,
                    job_type = %job.job_type,
                    retry_count = job.retry_count,
                    error = %error_msg,
                    "Job failed after max retries"
                );
                let _ = job_registry.mark_failed(&job.id, error_msg).await;
            }
        }
    }

    job_registry.clear_handle(&job.id).await;
    in_flight.decrement();
}
