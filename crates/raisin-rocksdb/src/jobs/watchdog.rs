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
    /// For each running job, checks if the heartbeat is too old or missing.
    /// Timed-out jobs are marked as failed and their cancel tokens are triggered.
    async fn check_timeouts(&self) -> Result<()> {
        let jobs = self.job_registry.list_jobs().await;
        let now = Utc::now();

        for job in jobs {
            // Check both Running and Executing jobs for timeouts
            if !matches!(job.status, JobStatus::Running | JobStatus::Executing) {
                continue;
            }

            // Check if job has a heartbeat
            let timed_out = if let Some(last_heartbeat) = job.last_heartbeat {
                let elapsed = now.signed_duration_since(last_heartbeat).num_seconds() as u64;
                if elapsed > job.timeout_seconds {
                    Some((elapsed, true))
                } else {
                    None
                }
            } else {
                // No heartbeat recorded - check how long it's been running
                let running_time = now.signed_duration_since(job.started_at).num_seconds() as u64;
                if running_time > job.timeout_seconds {
                    Some((running_time, false))
                } else {
                    None
                }
            };

            if let Some((elapsed, has_heartbeat)) = timed_out {
                let is_ai_tool_call = matches!(job.job_type, JobType::AIToolCallExecution { .. });

                if has_heartbeat {
                    tracing::warn!(
                        job_id = %job.id,
                        job_type = %job.job_type,
                        elapsed_seconds = elapsed,
                        timeout_seconds = job.timeout_seconds,
                        is_ai_tool_call = is_ai_tool_call,
                        "Job timed out, marking as failed"
                    );
                } else {
                    tracing::warn!(
                        job_id = %job.id,
                        job_type = %job.job_type,
                        running_seconds = elapsed,
                        timeout_seconds = job.timeout_seconds,
                        is_ai_tool_call = is_ai_tool_call,
                        "Job running without heartbeat, marking as failed"
                    );
                }

                let error_msg = if has_heartbeat {
                    format!(
                        "[timeout_final] Job timed out after {} seconds (last heartbeat: {}s ago)",
                        job.timeout_seconds, elapsed
                    )
                } else {
                    format!(
                        "[timeout_final] Job running for {} seconds without heartbeat (timeout: {}s)",
                        elapsed, job.timeout_seconds
                    )
                };

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

                // Try to cancel the job gracefully
                if let Some(token) = self.job_registry.get_cancel_token(&job.id).await {
                    token.cancel();
                }

                // Abort any running handler task so it cannot write a late completion.
                if self.job_registry.abort_handle(&job.id).await {
                    tracing::debug!(
                        job_id = %job.id,
                        "Aborted timed-out job handle"
                    );
                }

                // Invoke recovery callback (e.g., create error result nodes for AI tool calls)
                if let Some(ref cb) = self.on_timeout {
                    let job_clone = job.clone();
                    let error_clone = error_msg.clone();
                    let cb = cb.clone();
                    tokio::spawn(async move {
                        cb(job_clone, error_clone).await;
                    });
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_storage::jobs::JobType;

    #[tokio::test]
    async fn test_watchdog_detects_timeout() {
        let registry = Arc::new(JobRegistry::new());

        // Register a job
        let job_id = registry
            .register_job(JobType::IntegrityScan, None, None, None, None)
            .await
            .unwrap();

        // Mark as running
        registry.mark_running(&job_id).await.unwrap();

        // Set a heartbeat in the past (simulate timeout) - 400 seconds ago with 300s timeout
        registry
            .set_heartbeat_for_test(&job_id, Some(Utc::now() - chrono::Duration::seconds(400)))
            .await
            .unwrap();

        // Create watchdog with immediate shutdown
        let shutdown = CancellationToken::new();
        let watchdog = TimeoutWatchdog::new(registry.clone(), shutdown.clone());

        // Run one check
        watchdog.check_timeouts().await.unwrap();

        // Job should be marked as failed
        let status = registry.get_status(&job_id).await.unwrap();
        assert!(matches!(status, JobStatus::Failed(_)));
    }
}
