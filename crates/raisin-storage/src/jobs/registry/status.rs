// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Job status and progress update methods for the JobRegistry.
//!
//! Contains methods for updating job status, progress, results,
//! claiming jobs for execution, and marking jobs as completed or failed.

use chrono::Utc;
use tokio::task::JoinHandle;

use super::JobRegistry;
use crate::jobs::{JobEvent, JobId, JobInfo, JobStatus};
use raisin_error::{Error as RaisinError, Result};

impl JobRegistry {
    /// Update job status
    pub async fn update_status(&self, job_id: &JobId, status: JobStatus) -> Result<()> {
        let is_terminal = matches!(
            status,
            JobStatus::Completed | JobStatus::Failed(_) | JobStatus::Cancelled
        );

        let (old_status, job_info) = {
            let mut jobs = self.jobs.write().await;
            if let Some(job) = jobs.get_mut(job_id) {
                let old = job.status.clone();
                if matches!(
                    old,
                    JobStatus::Completed | JobStatus::Failed(_) | JobStatus::Cancelled
                ) {
                    if old == status {
                        // Idempotent terminal write; nothing to change.
                        return Ok(());
                    }
                    return Err(RaisinError::Validation(format!(
                        "Cannot transition terminal job {} from {:?} to {:?}",
                        job_id, old, status
                    )));
                }
                job.status = status.clone();
                if is_terminal {
                    job.completed_at = Some(Utc::now());
                }
                (Some(old), job.to_job_info())
            } else {
                return Err(RaisinError::NotFound(format!("Job {:?} not found", job_id)));
            }
        };

        // Clean up dedup key when job reaches terminal state
        // This allows new jobs with the same key to be registered
        if is_terminal {
            let mut dedup_keys = self.dedup_keys.write().await;
            // Find and remove any dedup key pointing to this job
            dedup_keys.retain(|_, v| v != job_id);
        }

        // Persist after updating in-memory state
        if let Some(persistence) = &self.persistence {
            if let Err(e) = persistence.persist_job(job_id, &job_info).await {
                tracing::error!(
                    job_id = %job_id,
                    error = %e,
                    "Failed to persist job status update"
                );
                // Don't fail the operation - in-memory state is updated
            }
        }

        // Emit status change event
        let event = JobEvent {
            job_id: job_id.clone(),
            job_info,
            old_status,
            new_status: status,
            timestamp: Utc::now(),
        };
        self.monitors.broadcast_update(event).await;

        Ok(())
    }

    /// Update job progress
    pub async fn update_progress(&self, job_id: &JobId, progress: f32) -> Result<()> {
        {
            let mut jobs = self.jobs.write().await;
            if let Some(job) = jobs.get_mut(job_id) {
                job.progress = Some(progress);
            } else {
                return Err(RaisinError::NotFound(format!("Job {:?} not found", job_id)));
            }
        }

        // Emit progress event
        self.monitors.broadcast_progress(job_id, progress).await;

        Ok(())
    }

    /// Set job result data
    pub async fn set_result(&self, job_id: &JobId, result: serde_json::Value) -> Result<()> {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.get_mut(job_id) {
            job.result = Some(result);
            Ok(())
        } else {
            Err(RaisinError::NotFound(format!("Job {:?} not found", job_id)))
        }
    }

    /// Mark job as running
    pub async fn mark_running(&self, job_id: &JobId) -> Result<()> {
        self.update_status(job_id, JobStatus::Running).await
    }

    /// Try to claim a job for processing (atomic compare-and-swap)
    ///
    /// Only succeeds if job exists AND is currently in Scheduled status.
    /// This prevents race conditions where multiple workers could pick up the same job.
    ///
    /// # Returns
    ///
    /// * `Ok(true)` - Job was successfully claimed (now Running)
    /// * `Ok(false)` - Job was already claimed by another worker or not ready for retry
    /// * `Err(e)` - Job not found or other error
    pub async fn try_claim_job(&self, job_id: &JobId) -> Result<bool> {
        let (claimed, job_info) = {
            let mut jobs = self.jobs.write().await;
            if let Some(job) = jobs.get_mut(job_id) {
                // Only claim if currently Scheduled
                if !matches!(job.status, JobStatus::Scheduled) {
                    return Ok(false); // Already claimed by another worker
                }

                // Check retry delay - don't claim if not ready yet
                if let Some(next_retry) = job.next_retry_at {
                    if chrono::Utc::now() < next_retry {
                        return Ok(false); // Not ready for retry yet
                    }
                }

                // Atomically set to Running
                job.status = JobStatus::Running;
                job.last_heartbeat = Some(chrono::Utc::now());
                (true, job.to_job_info())
            } else {
                return Err(RaisinError::NotFound(format!("Job {:?} not found", job_id)));
            }
        };

        if claimed {
            // Persist after updating in-memory state
            if let Some(persistence) = &self.persistence {
                if let Err(e) = persistence.persist_job(job_id, &job_info).await {
                    tracing::error!(
                        job_id = %job_id,
                        error = %e,
                        "Failed to persist job claim"
                    );
                    // Don't fail the operation - in-memory state is updated
                }
            }

            // Emit status change event
            let event = JobEvent {
                job_id: job_id.clone(),
                job_info,
                old_status: Some(JobStatus::Scheduled),
                new_status: JobStatus::Running,
                timestamp: chrono::Utc::now(),
            };
            self.monitors.broadcast_update(event).await;
        }

        Ok(claimed)
    }

    /// Mark job as executing (handler task spawned and actively running)
    pub async fn mark_executing(&self, job_id: &JobId) -> Result<()> {
        self.update_status(job_id, JobStatus::Executing).await
    }

    /// Mark job as completed
    pub async fn mark_completed(&self, job_id: &JobId) -> Result<()> {
        self.update_status(job_id, JobStatus::Completed).await
    }

    /// Mark job as failed
    pub async fn mark_failed(&self, job_id: &JobId, error: String) -> Result<()> {
        // First, update the status and broadcast the event
        self.update_status(job_id, JobStatus::Failed(error.clone()))
            .await?;

        // Then, set the error field (update_status doesn't do this)
        let job_info = {
            let mut jobs = self.jobs.write().await;
            if let Some(job) = jobs.get_mut(job_id) {
                job.error = Some(error);
                job.to_job_info()
            } else {
                return Ok(()); // Job already removed
            }
        };

        // Persist the error field update
        if let Some(persistence) = &self.persistence {
            if let Err(e) = persistence.persist_job(job_id, &job_info).await {
                tracing::error!(
                    job_id = %job_id,
                    error = %e,
                    "Failed to persist job error field"
                );
            }
        }

        Ok(())
    }

    /// Register or replace the async handle for a running job.
    pub async fn set_handle(&self, job_id: &JobId, handle: JoinHandle<()>) -> Result<()> {
        {
            let jobs = self.jobs.read().await;
            if !jobs.contains_key(job_id) {
                return Err(RaisinError::NotFound(format!("Job {:?} not found", job_id)));
            }
        }

        let mut handles = self.handles.write().await;
        handles.insert(job_id.clone(), handle);
        Ok(())
    }

    /// Remove and abort a running async handle, if present.
    pub async fn abort_handle(&self, job_id: &JobId) -> bool {
        let mut handles = self.handles.write().await;
        if let Some(handle) = handles.remove(job_id) {
            handle.abort();
            return true;
        }
        false
    }

    /// Remove a stored async handle when a job exits normally.
    pub async fn clear_handle(&self, job_id: &JobId) {
        let mut handles = self.handles.write().await;
        handles.remove(job_id);
    }

    /// Schedule a job retry after failure
    ///
    /// Increments retry count, resets status to Scheduled, and stores the error.
    /// The job will be picked up again by a worker for another execution attempt.
    ///
    /// # Arguments
    ///
    /// * `job_id` - Job identifier
    /// * `error` - Error message from the failed attempt
    ///
    /// # Errors
    ///
    /// Returns an error if the job is not found or persistence fails
    pub async fn schedule_retry(&self, job_id: &JobId, error: String) -> Result<()> {
        let (job_info, old_status) = {
            let mut jobs = self.jobs.write().await;

            if let Some(job) = jobs.get_mut(job_id) {
                if matches!(
                    job.status,
                    JobStatus::Completed | JobStatus::Failed(_) | JobStatus::Cancelled
                ) {
                    return Err(RaisinError::Validation(format!(
                        "Cannot schedule retry for terminal job {} in status {:?}",
                        job_id, job.status
                    )));
                }

                let old = job.status.clone();
                job.retry_count += 1;
                job.status = JobStatus::Scheduled;
                job.error = Some(error.clone());
                job.last_heartbeat = None; // Reset heartbeat for fresh retry

                // Calculate exponential backoff: 10s, 30s, 60s
                let delay_seconds = match job.retry_count {
                    1 => 10, // First retry after 10 seconds
                    2 => 30, // Second retry after 30 seconds
                    _ => 60, // Third+ retry after 60 seconds
                };
                job.next_retry_at = Some(Utc::now() + chrono::Duration::seconds(delay_seconds));

                tracing::info!(
                    job_id = %job_id,
                    retry_count = job.retry_count,
                    delay_seconds = delay_seconds,
                    next_retry_at = ?job.next_retry_at,
                    "Scheduled job retry with exponential backoff"
                );

                (job.to_job_info(), Some(old))
            } else {
                return Err(RaisinError::NotFound(format!("Job {:?} not found", job_id)));
            }
        };

        // Persist retry state
        if let Some(persistence) = &self.persistence {
            if let Err(e) = persistence.persist_job(job_id, &job_info).await {
                tracing::error!(
                    job_id = %job_id,
                    error = %e,
                    "Failed to persist retry state"
                );
                return Err(e);
            }
        }

        // Emit event for retry
        let event = JobEvent {
            job_id: job_id.clone(),
            job_info,
            old_status,
            new_status: JobStatus::Scheduled,
            timestamp: Utc::now(),
        };
        self.monitors.broadcast_update(event).await;

        Ok(())
    }
}
