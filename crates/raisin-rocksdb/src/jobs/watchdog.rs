//! Timeout watchdog for detecting and killing stuck jobs
//!
//! Runs as a background task, checking job heartbeats every 15 seconds.
//! If a job's last_heartbeat is older than its timeout_seconds, the job
//! is marked as failed and its cancellation token is triggered.
//!
//! For AIToolCallExecution jobs, a recovery callback can be registered
//! to create error result nodes so the agent loop doesn't hang.

use chrono::Utc;
use raisin_error::Result;
use raisin_storage::jobs::{JobInfo, JobRegistry, JobStatus, JobType};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Callback invoked when a job times out. Receives the job info and error message.
/// Used to perform recovery actions (e.g., creating error result nodes for AI tool calls).
pub type OnJobTimeoutFn =
    Arc<dyn Fn(JobInfo, String) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Background task that monitors job heartbeats and kills timed-out jobs
///
/// The watchdog runs every 15 seconds and checks all running jobs. If a job's
/// heartbeat is too old or missing, the job is marked as failed and cancelled.
///
/// An optional `on_timeout` callback can be registered to perform recovery
/// actions for specific job types (e.g., creating error result nodes for
/// AIToolCallExecution jobs).
pub struct TimeoutWatchdog {
    job_registry: Arc<JobRegistry>,
    shutdown: CancellationToken,
    on_timeout: Option<OnJobTimeoutFn>,
}

impl TimeoutWatchdog {
    /// Create a new timeout watchdog
    ///
    /// # Arguments
    ///
    /// * `job_registry` - Shared job registry to monitor
    /// * `shutdown` - Cancellation token for graceful shutdown
    pub fn new(job_registry: Arc<JobRegistry>, shutdown: CancellationToken) -> Self {
        Self {
            job_registry,
            shutdown,
            on_timeout: None,
        }
    }

    /// Register a callback to be invoked when a job times out.
    ///
    /// The callback receives the `JobInfo` and error message. For
    /// `AIToolCallExecution` jobs, this can be used to create error
    /// result nodes so the agent loop continues with an error message
    /// instead of hanging forever.
    pub fn with_timeout_callback(mut self, callback: OnJobTimeoutFn) -> Self {
        self.on_timeout = Some(callback);
        self
    }

    /// Run the watchdog loop
    ///
    /// Continuously monitors jobs until the shutdown signal is received.
    /// Checks heartbeats every 15 seconds.
    pub async fn run(self) {
        tracing::info!("Timeout watchdog started");

        loop {
            tokio::select! {
                _ = self.shutdown.cancelled() => {
                    tracing::info!("Timeout watchdog received shutdown signal");
                    break;
                }
                _ = tokio::time::sleep(Duration::from_secs(15)) => {
                    if let Err(e) = self.check_timeouts().await {
                        tracing::error!(error = %e, "Timeout check failed");
                    }
                }
            }
        }

        tracing::info!("Timeout watchdog stopped");
    }

    /// Check all running jobs for timeouts
    ///
    /// A job is considered timed out when either:
    /// 1. Its total execution time (since the current attempt was claimed)
    ///    exceeds `timeout_seconds`. This is the primary check: the worker
    ///    auto-renews heartbeats from an independent task, so a handler that
    ///    is stuck (e.g. parked in a blocking call) keeps a fresh heartbeat
    ///    forever and would never be reaped by heartbeat staleness alone.
    /// 2. Its heartbeat is older than `timeout_seconds` (heartbeat task died).
    ///
    /// Timed-out jobs follow normal retry semantics: they are scheduled for
    /// retry while attempts remain, and marked failed once retries are
    /// exhausted. In both cases the stuck attempt is cancelled and aborted.
    async fn check_timeouts(&self) -> Result<()> {
        let jobs = self.job_registry.list_jobs().await;
        let now = Utc::now();

        for job in jobs {
            // Check both Running and Executing jobs for timeouts
            if !matches!(job.status, JobStatus::Running | JobStatus::Executing) {
                continue;
            }

            // Wall-clock execution time of the current attempt.
            // Fall back to started_at for jobs claimed before executing_since
            // existed (restored from older persisted state).
            let exec_start = job.executing_since.unwrap_or(job.started_at);
            let running_secs = now.signed_duration_since(exec_start).num_seconds().max(0) as u64;

            // Heartbeat staleness (catches dead heartbeat tasks quickly).
            let heartbeat_stale_secs = job
                .last_heartbeat
                .map(|hb| now.signed_duration_since(hb).num_seconds().max(0) as u64);

            let exceeded_runtime = running_secs > job.timeout_seconds;
            let exceeded_heartbeat = heartbeat_stale_secs
                .map(|s| s > job.timeout_seconds)
                .unwrap_or(false);

            if !exceeded_runtime && !exceeded_heartbeat {
                continue;
            }

            let is_ai_tool_call = matches!(job.job_type, JobType::AIToolCallExecution { .. });
            let retries_left = job.retry_count < job.max_retries;

            tracing::warn!(
                job_id = %job.id,
                job_type = %job.job_type,
                running_seconds = running_secs,
                heartbeat_stale_seconds = ?heartbeat_stale_secs,
                timeout_seconds = job.timeout_seconds,
                retry_count = job.retry_count,
                max_retries = job.max_retries,
                will_retry = retries_left,
                is_ai_tool_call = is_ai_tool_call,
                "Job timed out"
            );

            // Cancel the stuck attempt gracefully first...
            if let Some(token) = self.job_registry.get_cancel_token(&job.id).await {
                token.cancel();
            }

            // ...then abort the handler task so it cannot write a late completion.
            if self.job_registry.abort_handle(&job.id).await {
                tracing::debug!(
                    job_id = %job.id,
                    "Aborted timed-out job handle"
                );
            }

            if retries_left {
                // Normal retry semantics: re-schedule with backoff. The
                // registry resets heartbeat/execution tracking and issues a
                // fresh cancel token for the next attempt.
                let error_msg = format!(
                    "[timeout] Job execution exceeded {}s (running for {}s), scheduling retry",
                    job.timeout_seconds, running_secs
                );
                if let Err(e) = self.job_registry.schedule_retry(&job.id, error_msg).await {
                    tracing::warn!(
                        job_id = %job.id,
                        error = %e,
                        "Failed to schedule retry for timed-out job (continuing watchdog scan)"
                    );
                }
                continue;
            }

            let error_msg = format!(
                "[timeout_final] Job timed out after {}s (running for {}s, retries exhausted)",
                job.timeout_seconds, running_secs
            );

            if let Err(e) = self
                .job_registry
                .mark_failed(&job.id, error_msg.clone())
                .await
            {
                tracing::warn!(
                    job_id = %job.id,
                    error = %e,
                    "Failed to mark timed-out job as failed (continuing watchdog scan)"
                );
                continue;
            }

            // Invoke recovery callback (e.g., create error result nodes for AI
            // tool calls) only on final failure.
            if let Some(ref cb) = self.on_timeout {
                let job_clone = job.clone();
                let error_clone = error_msg.clone();
                let cb = cb.clone();
                tokio::spawn(async move {
                    cb(job_clone, error_clone).await;
                });
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_storage::jobs::JobType;

    /// Register a job, mark it running, and return (registry, job_id, timeout).
    async fn running_job(max_retries: u32) -> (Arc<JobRegistry>, raisin_storage::jobs::JobId, i64) {
        let registry = Arc::new(JobRegistry::new());

        let job_id = registry
            .register_job(
                JobType::IntegrityScan,
                "test-tenant".to_string(),
                None,
                None,
                Some(max_retries),
            )
            .await
            .unwrap();

        registry.mark_running(&job_id).await.unwrap();

        let timeout = registry
            .get_job_info(&job_id)
            .await
            .unwrap()
            .timeout_seconds as i64;

        (registry, job_id, timeout)
    }

    #[tokio::test]
    async fn test_watchdog_detects_timeout() {
        // max_retries = 0 so the timeout is final (Failed, not retried)
        let (registry, job_id, timeout) = running_job(0).await;

        // Set a heartbeat older than the job's timeout (simulate dead heartbeat task)
        registry
            .set_heartbeat_for_test(
                &job_id,
                Some(Utc::now() - chrono::Duration::seconds(timeout + 100)),
            )
            .await
            .unwrap();
        // Match the heartbeat: the execution attempt also started back then
        registry
            .set_execution_start_for_test(
                &job_id,
                Some(Utc::now() - chrono::Duration::seconds(timeout + 100)),
            )
            .await
            .unwrap();

        let shutdown = CancellationToken::new();
        let watchdog = TimeoutWatchdog::new(registry.clone(), shutdown.clone());

        watchdog.check_timeouts().await.unwrap();

        // Job should be marked as failed
        let status = registry.get_status(&job_id).await.unwrap();
        assert!(matches!(status, JobStatus::Failed(_)));
    }

    #[tokio::test]
    async fn test_watchdog_reaps_stuck_job_with_fresh_heartbeat() {
        // The reap path: a stuck Executing job whose heartbeat task is alive
        // (heartbeat is fresh) must still be reaped once total execution
        // time exceeds the timeout.
        let (registry, job_id, timeout) = running_job(0).await;
        registry.mark_executing(&job_id).await.unwrap();

        // Heartbeat is fresh (auto-renewed by the worker's heartbeat task)...
        registry
            .set_heartbeat_for_test(&job_id, Some(Utc::now()))
            .await
            .unwrap();
        // ...but the attempt has been executing far past its timeout.
        registry
            .set_execution_start_for_test(
                &job_id,
                Some(Utc::now() - chrono::Duration::seconds(timeout + 100)),
            )
            .await
            .unwrap();

        let shutdown = CancellationToken::new();
        let watchdog = TimeoutWatchdog::new(registry.clone(), shutdown.clone());

        watchdog.check_timeouts().await.unwrap();

        let status = registry.get_status(&job_id).await.unwrap();
        assert!(
            matches!(status, JobStatus::Failed(_)),
            "stuck job with fresh heartbeat must be reaped, got {:?}",
            status
        );
    }

    #[tokio::test]
    async fn test_watchdog_schedules_retry_when_attempts_remain() {
        // Normal retry semantics: a timed-out job with retries left goes
        // back to Scheduled (with backoff), not to Failed.
        let (registry, job_id, timeout) = running_job(3).await;

        registry
            .set_heartbeat_for_test(&job_id, Some(Utc::now()))
            .await
            .unwrap();
        registry
            .set_execution_start_for_test(
                &job_id,
                Some(Utc::now() - chrono::Duration::seconds(timeout + 100)),
            )
            .await
            .unwrap();

        let shutdown = CancellationToken::new();
        let watchdog = TimeoutWatchdog::new(registry.clone(), shutdown.clone());

        watchdog.check_timeouts().await.unwrap();

        let info = registry.get_job_info(&job_id).await.unwrap();
        assert!(
            matches!(info.status, JobStatus::Scheduled),
            "timed-out job with retries left must be re-scheduled, got {:?}",
            info.status
        );
        assert_eq!(info.retry_count, 1);
        assert!(info.next_retry_at.is_some(), "retry backoff must be set");
        assert!(
            info.executing_since.is_none(),
            "execution tracking must reset for the retry"
        );
    }

    #[tokio::test]
    async fn test_watchdog_ignores_healthy_job() {
        let (registry, job_id, _timeout) = running_job(0).await;

        registry
            .set_heartbeat_for_test(&job_id, Some(Utc::now()))
            .await
            .unwrap();

        let shutdown = CancellationToken::new();
        let watchdog = TimeoutWatchdog::new(registry.clone(), shutdown.clone());

        watchdog.check_timeouts().await.unwrap();

        let status = registry.get_status(&job_id).await.unwrap();
        assert!(matches!(status, JobStatus::Running));
    }
}
