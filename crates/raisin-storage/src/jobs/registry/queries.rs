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

//! Job query and status retrieval methods for the JobRegistry.
//!
//! Contains methods for querying job status, listing jobs,
//! and waiting for job completion.

use chrono::{DateTime, Utc};
use std::sync::Arc;

use super::JobRegistry;
use crate::jobs::{JobId, JobInfo, JobStatus};
use raisin_error::{Error as RaisinError, Result};

impl JobRegistry {
    /// Emit a real-time log entry for a running job
    ///
    /// This broadcasts the log entry to all registered monitors (e.g., SSE clients)
    /// so logs can be streamed in real-time to the UI.
    pub async fn emit_log(&self, job_id: &JobId, level: &str, message: &str) {
        tracing::debug!(
            job_id = %job_id,
            level = %level,
            message = %message,
            "JobRegistry: emitting log to monitors"
        );
        let entry = crate::jobs::JobLogEntry {
            job_id: job_id.clone(),
            level: level.to_string(),
            message: message.to_string(),
            timestamp: Utc::now(),
        };
        self.monitors.broadcast_log(entry).await;
    }

    /// Get job status
    pub async fn get_status(&self, job_id: &JobId) -> Result<JobStatus> {
        let jobs = self.jobs.read().await;
        jobs.get(job_id)
            .map(|job| job.status.clone())
            .ok_or_else(|| RaisinError::NotFound(format!("Job {} not found", job_id)))
    }

    /// Get job info
    pub async fn get_job_info(&self, job_id: &JobId) -> Result<JobInfo> {
        let jobs = self.jobs.read().await;
        jobs.get(job_id)
            .map(|job| job.to_job_info())
            .ok_or_else(|| RaisinError::NotFound(format!("Job {} not found", job_id)))
    }

    /// List all jobs
    pub async fn list_jobs(&self) -> Vec<JobInfo> {
        let jobs = self.jobs.read().await;
        jobs.values().map(|job| job.to_job_info()).collect()
    }

    /// List jobs by tenant
    pub async fn list_jobs_by_tenant(&self, tenant: &str) -> Vec<JobInfo> {
        let jobs = self.jobs.read().await;
        jobs.values()
            .filter(|job| job.tenant.as_deref() == Some(tenant))
            .map(|job| job.to_job_info())
            .collect()
    }

    /// Check if a job is still running
    pub async fn is_running(&self, job_id: &JobId) -> bool {
        let jobs = self.jobs.read().await;
        jobs.get(job_id)
            .map(|job| {
                matches!(
                    job.status,
                    JobStatus::Running | JobStatus::Executing | JobStatus::Scheduled
                )
            })
            .unwrap_or(false)
    }

    /// Wait for a job to complete
    pub async fn wait_for_completion(&self, job_id: &JobId) -> Result<JobStatus> {
        // Poll the job status until it's no longer running
        loop {
            let status = self.get_status(job_id).await?;
            match status {
                JobStatus::Running | JobStatus::Executing | JobStatus::Scheduled => {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                }
                _ => return Ok(status),
            }
        }
    }

    /// Update job heartbeat timestamp
    ///
    /// Called periodically by workers to indicate the job is still making progress.
    /// Used by the timeout watchdog to detect stuck/crashed jobs.
    ///
    /// # Arguments
    ///
    /// * `job_id` - Job identifier
    ///
    /// # Errors
    ///
    /// Note: Returns Ok(()) even if job is not found - the job likely completed
    /// and was removed from the registry. This is expected behavior due to the
    /// race between job completion and the heartbeat task stopping.
    pub async fn update_heartbeat(&self, job_id: &JobId) -> Result<()> {
        let job_info = {
            let mut jobs = self.jobs.write().await;
            if let Some(job) = jobs.get_mut(job_id) {
                if matches!(
                    job.status,
                    JobStatus::Completed | JobStatus::Failed(_) | JobStatus::Cancelled
                ) {
                    return Ok(());
                }
                job.last_heartbeat = Some(Utc::now());
                Some(job.to_job_info())
            } else {
                // Job not found - likely already completed and removed
                // This is expected, not an error
                tracing::trace!(job_id = %job_id, "Heartbeat for completed/removed job");
                None
            }
        };

        // Persist heartbeat update (only if job was found)
        if let Some(info) = job_info {
            if let Some(persistence) = &self.persistence {
                if let Err(e) = persistence.persist_job(job_id, &info).await {
                    tracing::error!(
                        job_id = %job_id,
                        error = %e,
                        "Failed to persist heartbeat update"
                    );
                    // Don't fail - heartbeat is best-effort
                }
            }
        }

        Ok(())
    }

    /// Get the cancellation token for a job
    ///
    /// Used by the timeout watchdog to gracefully cancel stuck jobs.
    ///
    /// # Arguments
    ///
    /// * `job_id` - Job identifier
    ///
    /// # Returns
    ///
    /// The job's cancellation token if it exists, None otherwise
    pub async fn get_cancel_token(
        &self,
        job_id: &JobId,
    ) -> Option<Arc<tokio_util::sync::CancellationToken>> {
        let jobs = self.jobs.read().await;
        jobs.get(job_id).and_then(|job| job.cancel_token.clone())
    }

    /// Set heartbeat to a specific time (for testing timeout scenarios)
    ///
    /// # Safety
    ///
    /// This method is intended for testing only. It directly manipulates the
    /// internal job state without triggering any normal heartbeat logic.
    pub async fn set_heartbeat_for_test(
        &self,
        job_id: &JobId,
        heartbeat: Option<DateTime<Utc>>,
    ) -> Result<()> {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.get_mut(job_id) {
            job.last_heartbeat = heartbeat;
            Ok(())
        } else {
            Err(RaisinError::NotFound(format!("Job {:?} not found", job_id)))
        }
    }
}
