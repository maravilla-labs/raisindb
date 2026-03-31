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

//! Job lifecycle management methods for the JobRegistry.
//!
//! Contains methods for cancelling, deleting, restoring,
//! and cleaning up jobs.

use chrono::Utc;

use super::job_entry::JobEntry;
use super::JobRegistry;
use crate::jobs::{JobId, JobInfo, JobStatus};
use raisin_error::{Error as RaisinError, Result};

impl JobRegistry {
    /// Cancel a job
    pub async fn cancel_job(&self, job_id: &JobId) -> Result<()> {
        // First, try to use the cancellation token if available
        {
            let jobs = self.jobs.read().await;
            if let Some(job) = jobs.get(job_id) {
                if let Some(token) = &job.cancel_token {
                    token.cancel();
                }
            }
        }

        // Then abort the handle if available
        let mut handles = self.handles.write().await;
        if let Some(handle) = handles.remove(job_id) {
            handle.abort();
        }

        // Update status
        self.update_status(job_id, JobStatus::Cancelled).await?;

        Ok(())
    }

    /// Delete a job from the registry
    /// This permanently removes the job and should only be used for completed/cancelled jobs
    pub async fn delete_job(&self, job_id: &JobId) -> Result<()> {
        // Get job info before deletion for notification
        let job_info = {
            let jobs = self.jobs.read().await;
            jobs.get(job_id)
                .map(|job| job.to_job_info())
                .ok_or_else(|| RaisinError::NotFound(format!("Job {:?} not found", job_id)))?
        };

        // Don't allow deletion of active jobs
        match job_info.status {
            JobStatus::Running | JobStatus::Executing | JobStatus::Scheduled => {
                return Err(RaisinError::Validation(
                    "Cannot delete a running or scheduled job. Cancel it first.".to_string(),
                ));
            }
            _ => {}
        }

        // Remove from both maps
        let mut jobs = self.jobs.write().await;
        let mut handles = self.handles.write().await;

        jobs.remove(job_id);
        handles.remove(job_id);

        // Notify monitors that job was removed
        self.monitors.broadcast_removed(job_id).await;

        Ok(())
    }

    /// Delete multiple jobs from the registry in a single operation
    ///
    /// This is more efficient than calling `delete_job` in a loop because it:
    /// - Acquires locks only once for the batch
    /// - Skips running/scheduled jobs without failing the entire operation
    ///
    /// # Arguments
    ///
    /// * `job_ids` - List of job IDs to delete
    ///
    /// # Returns
    ///
    /// A tuple of (deleted_count, skipped_count) where skipped jobs are those
    /// that were running, scheduled, or not found
    pub async fn delete_jobs_batch(&self, job_ids: &[JobId]) -> (usize, usize) {
        let mut deleted_count = 0;
        let mut skipped_count = 0;
        let mut deleted_ids = Vec::new();

        // Single lock acquisition for the entire batch
        {
            let mut jobs = self.jobs.write().await;
            let mut handles = self.handles.write().await;

            tracing::debug!(
                requested_ids = job_ids.len(),
                registry_size = jobs.len(),
                "Batch delete starting"
            );

            for job_id in job_ids {
                // Check if job exists and is deletable
                match jobs.get(job_id) {
                    Some(job) => {
                        let can_delete = !matches!(
                            job.status,
                            JobStatus::Running | JobStatus::Executing | JobStatus::Scheduled
                        );
                        if can_delete {
                            jobs.remove(job_id);
                            handles.remove(job_id);
                            deleted_ids.push(job_id.clone());
                            deleted_count += 1;
                        } else {
                            tracing::debug!(
                                job_id = %job_id,
                                status = ?job.status,
                                "Skipping job - still running/scheduled"
                            );
                            skipped_count += 1;
                        }
                    }
                    None => {
                        tracing::debug!(
                            job_id = %job_id,
                            "Skipping job - not found in registry"
                        );
                        skipped_count += 1;
                    }
                }
            }
        }

        // Notify monitors outside of lock
        for job_id in &deleted_ids {
            self.monitors.broadcast_removed(job_id).await;
        }

        (deleted_count, skipped_count)
    }

    /// Restore a job to the registry from persistent storage
    ///
    /// Used during crash recovery to reload jobs that were scheduled or running
    /// when the server shut down. The job is restored without a cancel token,
    /// which will be created when the job is actually executed.
    ///
    /// # Arguments
    ///
    /// * `job_info` - Job metadata loaded from persistent storage
    ///
    /// # Errors
    ///
    /// Returns an error if the job already exists in the registry
    pub async fn restore_job(&self, job_info: JobInfo) -> Result<()> {
        let entry = JobEntry {
            id: job_info.id.clone(),
            job_type: job_info.job_type.clone(),
            status: job_info.status.clone(),
            tenant: job_info.tenant.clone(),
            started_at: job_info.started_at,
            completed_at: job_info.completed_at,
            error: job_info.error.clone(),
            progress: job_info.progress,
            cancel_token: None, // Will be created fresh when job is executed
            result: job_info.result.clone(),
            retry_count: job_info.retry_count,
            max_retries: job_info.max_retries,
            last_heartbeat: job_info.last_heartbeat,
            timeout_seconds: job_info.timeout_seconds,
            next_retry_at: job_info.next_retry_at,
        };

        let mut jobs = self.jobs.write().await;
        jobs.insert(job_info.id.clone(), entry);

        tracing::debug!(
            job_id = %job_info.id,
            job_type = %job_info.job_type,
            status = ?job_info.status,
            retry_count = job_info.retry_count,
            "Restored job from persistent storage"
        );

        Ok(())
    }

    /// Clean up completed jobs older than the specified duration
    pub async fn cleanup_old_jobs(&self, max_age: chrono::Duration) {
        let cutoff = Utc::now() - max_age;
        let mut jobs = self.jobs.write().await;
        let mut handles = self.handles.write().await;

        let old_jobs: Vec<JobId> = jobs
            .iter()
            .filter(|(_, job)| {
                if let Some(completed) = job.completed_at {
                    completed < cutoff
                } else {
                    false
                }
            })
            .map(|(id, _)| id.clone())
            .collect();

        for job_id in old_jobs {
            jobs.remove(&job_id);
            handles.remove(&job_id);
        }
    }
}
