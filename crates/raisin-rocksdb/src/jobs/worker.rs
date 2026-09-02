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

                        // Load job context before spawning. Callers follow the
                        // register_job() -> JobDataStore.put() convention, so a
                        // fast claim can race the context write - retry briefly
                        // before declaring the job dead.
                        let mut context_opt = self.job_data_store.get(&job.tenant, &job.id)?;
                        if context_opt.is_none() {
                            for attempt in 1..=10u32 {
                                tokio::time::sleep(std::time::Duration::from_millis(
                                    50 * attempt as u64,
                                ))
                                .await;
                                context_opt = self.job_data_store.get(&job.tenant, &job.id)?;
                                if context_opt.is_some() {
                                    tracing::debug!(
                                        worker_id = self.id,
                                        job_id = %job.id,
                                        attempt,
                                        "Job context arrived after retry"
                                    );
                                    break;
                                }
                            }
                        }
                        let context = match context_opt {
                            Some(ctx) => ctx,
                            None => {
                                let error_msg = format!("Job context not found for job {}", job.id);
                                tracing::error!(
                                    worker_id = self.id,
                                    job_id = %job.id,
                                    "Missing job context (after grace retries)"
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

/// RAII cleanup for handler tasks.
///
/// Runs on every exit path of `execute_handler_task` - including when the
/// task is **aborted** by the timeout watchdog (abort drops the future, which
/// drops this guard). Without it, an aborted handler leaked its heartbeat
/// sub-task forever and never decremented the in-flight counter.
struct HandlerCleanupGuard {
    job_id: JobId,
    job_registry: Arc<JobRegistry>,
    heartbeat_token: CancellationToken,
    in_flight: InFlightTracker,
}

impl Drop for HandlerCleanupGuard {
    fn drop(&mut self) {
        self.heartbeat_token.cancel();
        self.in_flight.decrement();

        // clear_handle is async; spawn it if a runtime is still available.
        // (After watchdog abort_handle this is a harmless no-op.)
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            let registry = self.job_registry.clone();
            let job_id = self.job_id.clone();
            rt.spawn(async move {
                registry.clear_handle(&job_id).await;
            });
        }
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
                        // Self-terminate when the job is no longer active
                        // (terminal, or re-scheduled by the watchdog). This is
                        // the backstop for handler tasks that were aborted
                        // without unwinding (e.g. stuck in a blocking call).
                        if !job_registry_job_is_active(&registry, &job_id).await {
                            tracing::debug!(
                                job_id = %job_id,
                                "Heartbeat task exiting - job no longer active"
                            );
                            break;
                        }
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

    // All exit paths (including watchdog abort) cancel the heartbeat task,
    // decrement in-flight, and clear the stored handle via this guard.
    let _cleanup = HandlerCleanupGuard {
        job_id: job.id.clone(),
        job_registry: job_registry.clone(),
        heartbeat_token: heartbeat_token.clone(),
        in_flight: in_flight.clone(),
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
            return; // guard performs cleanup
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
        return; // guard performs cleanup
    }

    // Dispatch to handler
    let dispatch_result = handlers.dispatch(&job, &context).await;

    // Stop heartbeat before handling result so no late heartbeat is written
    // after the terminal status. (The guard would also cancel it on drop.)
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
            if let Err(e) = job_data_store.delete(&job.tenant, &job.id) {
                tracing::warn!(
                    job_id = %job.id,
                    error = %e,
                    "Failed to delete job context after completion"
                );
            }
        }
        Err(e) => {
            let error_msg = extract_clean_error_message(&e);

            // PARK, don't retry: the handler declined to attempt the work at
            // all because an upstream's circuit breaker is open.
            //
            // This is the job system's answer to the question the flow runtime
            // answers with `FlowError::is_infrastructural` — an error about the
            // DELIVERY of work rather than about the work must be redelivered,
            // not counted against it. Retrying here would spend the job's whole
            // budget (1 + 3 attempts, ~2 minutes) inside an outage that lasts
            // longer, and turn a recoverable backlog into permanent failures
            // across every tenant at once. Nothing was attempted, so nothing
            // failed.
            //
            // The permit is already released by the time this runs, and the
            // handler returned before doing any of the expensive work, so a
            // parked job costs one config read and one decrypt — not a chunked
            // document.
            if e.is_upstream_unavailable() {
                let retry_after = e
                    .upstream_retry_after()
                    .unwrap_or_else(|| std::time::Duration::from_secs(30));
                tracing::info!(
                    job_id = %job.id,
                    job_type = %job.job_type,
                    retry_count = job.retry_count,
                    retry_after_secs = retry_after.as_secs(),
                    reason = %error_msg,
                    "Upstream unavailable, parking job without consuming a retry"
                );
                if let Err(park_err) = job_registry.park_job(&job.id, error_msg, retry_after).await
                {
                    tracing::warn!(
                        job_id = %job.id,
                        error = %park_err,
                        "Failed to park job"
                    );
                }
            } else if job.retry_count < job.max_retries {
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

                // Delete job context data (cleanup). A retry still needs the
                // context to run again, so the retry arm above deliberately
                // keeps it; once retries are exhausted nothing will ever read
                // it again, and leaving it behind would strand whatever the
                // context carries (e.g. a magic-link plaintext token) in the
                // store indefinitely.
                if let Err(e) = job_data_store.delete(&job.tenant, &job.id) {
                    tracing::warn!(
                        job_id = %job.id,
                        error = %e,
                        "Failed to delete job context after permanent failure"
                    );
                }
            }
        }
    }

    // Handle/heartbeat/in-flight cleanup happens in HandlerCleanupGuard::drop.
}

/// Whether a job is still actively executing (used by the heartbeat backstop).
async fn job_registry_job_is_active(registry: &JobRegistry, job_id: &JobId) -> bool {
    matches!(
        registry.get_status(job_id).await,
        Ok(JobStatus::Running | JobStatus::Executing)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Aborting a handler task must still cancel its heartbeat token and
    /// decrement the in-flight counter (via HandlerCleanupGuard::drop).
    /// Regression test for the watchdog-abort leak.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn abort_runs_handler_cleanup_guard() {
        let registry = Arc::new(JobRegistry::new());
        let job_id = registry
            .register_job(
                raisin_storage::jobs::JobType::IntegrityScan,
                "test-tenant".to_string(),
                None,
                None,
                Some(0),
            )
            .await
            .unwrap();

        let in_flight = InFlightTracker::new();
        let heartbeat_token = CancellationToken::new();

        in_flight.increment();
        let guard = HandlerCleanupGuard {
            job_id: job_id.clone(),
            job_registry: registry.clone(),
            heartbeat_token: heartbeat_token.clone(),
            in_flight: in_flight.clone(),
        };

        // Simulate a handler that hangs forever at an await point.
        let task = tokio::spawn(async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        });

        // Give the task a chance to start, then abort it (watchdog path).
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(in_flight.get(), 1);
        task.abort();
        let _ = task.await;

        assert_eq!(in_flight.get(), 0, "abort must decrement in-flight count");
        assert!(
            heartbeat_token.is_cancelled(),
            "abort must cancel the heartbeat token"
        );
    }
}
