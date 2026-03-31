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

//! Job registration methods for the JobRegistry.
//!
//! Contains methods for registering new jobs, including idempotent
//! registration with deduplication keys.

use chrono::Utc;
use std::sync::Arc;
use tokio::task::JoinHandle;

use super::job_entry::JobEntry;
use super::JobRegistry;
use crate::jobs::{JobId, JobStatus, JobType};
use raisin_error::Result;

impl JobRegistry {
    /// Register a new job
    ///
    /// # Arguments
    ///
    /// * `job_type` - Type of job being registered
    /// * `tenant` - Optional tenant identifier
    /// * `handle` - Optional task handle for the job
    /// * `cancel_token` - Optional cancellation token
    /// * `max_retries` - Optional maximum retry attempts (default: 3, use 0 for no retries)
    pub async fn register_job(
        &self,
        job_type: JobType,
        tenant: Option<String>,
        handle: Option<JoinHandle<()>>,
        cancel_token: Option<Arc<tokio_util::sync::CancellationToken>>,
        max_retries: Option<u32>,
    ) -> Result<JobId> {
        let job_id = JobId::new();
        let timeout_seconds = job_type.default_timeout_seconds();
        let entry = JobEntry {
            id: job_id.clone(),
            job_type,
            status: JobStatus::Scheduled,
            tenant,
            started_at: Utc::now(),
            completed_at: None,
            error: None,
            progress: None,
            cancel_token,
            result: None,
            retry_count: 0,
            max_retries: max_retries.unwrap_or(3),
            last_heartbeat: None,
            timeout_seconds,
            next_retry_at: None, // Process immediately
        };

        let job_info = entry.to_job_info();

        let mut jobs = self.jobs.write().await;
        jobs.insert(job_id.clone(), entry);

        if let Some(h) = handle {
            let mut handles = self.handles.write().await;
            handles.insert(job_id.clone(), h);
        }

        // Notify monitors of new job
        self.monitors.broadcast_created(&job_info).await;

        Ok(job_id)
    }

    /// Register a new job with deduplication key (idempotent)
    ///
    /// This method registers a job only if no active job (Scheduled or Running)
    /// with the same deduplication key already exists. This prevents duplicate
    /// job execution when events might be processed multiple times.
    ///
    /// # Arguments
    ///
    /// * `job_type` - Type of job being registered
    /// * `tenant` - Optional tenant identifier
    /// * `dedup_key` - Unique key for deduplication (e.g., tool_call_path)
    /// * `max_retries` - Optional maximum retry attempts (default: 3, use 0 for no retries)
    ///
    /// # Returns
    ///
    /// * `Ok(Some(job_id))` - New job was registered
    /// * `Ok(None)` - Job was skipped because an active job with the same key exists
    pub async fn register_job_idempotent(
        &self,
        job_type: JobType,
        tenant: Option<String>,
        dedup_key: String,
        max_retries: Option<u32>,
    ) -> Result<Option<JobId>> {
        // Check if an active job with this dedup key already exists
        {
            let dedup_keys = self.dedup_keys.read().await;
            if let Some(existing_job_id) = dedup_keys.get(&dedup_key) {
                // Check if the existing job is still active
                let jobs = self.jobs.read().await;
                if let Some(job) = jobs.get(existing_job_id) {
                    if matches!(
                        job.status,
                        JobStatus::Scheduled | JobStatus::Running | JobStatus::Executing
                    ) {
                        tracing::debug!(
                            dedup_key = %dedup_key,
                            existing_job_id = %existing_job_id,
                            status = ?job.status,
                            "Skipping duplicate job registration - active job exists"
                        );
                        return Ok(None);
                    }
                }
            }
        }

        // No active job with this key - register a new one
        let job_id = JobId::new();
        let timeout_seconds = job_type.default_timeout_seconds();
        let entry = JobEntry {
            id: job_id.clone(),
            job_type,
            status: JobStatus::Scheduled,
            tenant,
            started_at: Utc::now(),
            completed_at: None,
            error: None,
            progress: None,
            cancel_token: Some(Arc::new(tokio_util::sync::CancellationToken::new())),
            result: None,
            retry_count: 0,
            max_retries: max_retries.unwrap_or(3),
            last_heartbeat: None,
            timeout_seconds,
            next_retry_at: None, // Process immediately
        };

        let job_info = entry.to_job_info();

        // Insert job and update dedup key atomically
        {
            let mut jobs = self.jobs.write().await;
            let mut dedup_keys = self.dedup_keys.write().await;

            // Double-check in case of race condition
            if let Some(existing_job_id) = dedup_keys.get(&dedup_key) {
                if let Some(job) = jobs.get(existing_job_id) {
                    if matches!(
                        job.status,
                        JobStatus::Scheduled | JobStatus::Running | JobStatus::Executing
                    ) {
                        tracing::debug!(
                            dedup_key = %dedup_key,
                            existing_job_id = %existing_job_id,
                            "Race condition avoided - active job exists"
                        );
                        return Ok(None);
                    }
                }
            }

            jobs.insert(job_id.clone(), entry);
            dedup_keys.insert(dedup_key.clone(), job_id.clone());
        }

        tracing::debug!(
            job_id = %job_id,
            dedup_key = %dedup_key,
            "Registered job with deduplication key"
        );

        // Notify monitors of new job
        self.monitors.broadcast_created(&job_info).await;

        Ok(Some(job_id))
    }

    /// Register a new job with a pre-generated job ID (idempotent)
    ///
    /// This method is similar to `register_job_idempotent` but accepts a pre-generated
    /// job ID. This allows callers to store job context BEFORE registration, avoiding
    /// race conditions where the job is dispatched before its context is available.
    ///
    /// # Arguments
    ///
    /// * `job_id` - Pre-generated job ID
    /// * `job_type` - Type of job being registered
    /// * `tenant` - Optional tenant identifier
    /// * `dedup_key` - Unique key for deduplication
    /// * `max_retries` - Optional maximum retry attempts (default: 3)
    ///
    /// # Returns
    ///
    /// * `Ok(true)` - Job was registered
    /// * `Ok(false)` - Job was skipped because an active job with the same key exists
    pub async fn register_job_with_id_idempotent(
        &self,
        job_id: JobId,
        job_type: JobType,
        tenant: Option<String>,
        dedup_key: String,
        max_retries: Option<u32>,
    ) -> Result<bool> {
        // Check if an active job with this dedup key already exists
        {
            let dedup_keys = self.dedup_keys.read().await;
            if let Some(existing_job_id) = dedup_keys.get(&dedup_key) {
                let jobs = self.jobs.read().await;
                if let Some(job) = jobs.get(existing_job_id) {
                    if matches!(
                        job.status,
                        JobStatus::Scheduled | JobStatus::Running | JobStatus::Executing
                    ) {
                        tracing::debug!(
                            dedup_key = %dedup_key,
                            existing_job_id = %existing_job_id,
                            status = ?job.status,
                            "Skipping duplicate job registration - active job exists"
                        );
                        return Ok(false);
                    }
                }
            }
        }

        let timeout_seconds = job_type.default_timeout_seconds();
        let entry = JobEntry {
            id: job_id.clone(),
            job_type,
            status: JobStatus::Scheduled,
            tenant,
            started_at: Utc::now(),
            completed_at: None,
            error: None,
            progress: None,
            cancel_token: Some(Arc::new(tokio_util::sync::CancellationToken::new())),
            result: None,
            retry_count: 0,
            max_retries: max_retries.unwrap_or(3),
            last_heartbeat: None,
            timeout_seconds,
            next_retry_at: None,
        };

        let job_info = entry.to_job_info();

        // Insert job and update dedup key atomically
        {
            let mut jobs = self.jobs.write().await;
            let mut dedup_keys = self.dedup_keys.write().await;

            // Double-check in case of race condition
            if let Some(existing_job_id) = dedup_keys.get(&dedup_key) {
                if let Some(job) = jobs.get(existing_job_id) {
                    if matches!(
                        job.status,
                        JobStatus::Scheduled | JobStatus::Running | JobStatus::Executing
                    ) {
                        tracing::debug!(
                            dedup_key = %dedup_key,
                            existing_job_id = %existing_job_id,
                            "Race condition avoided - active job exists"
                        );
                        return Ok(false);
                    }
                }
            }

            jobs.insert(job_id.clone(), entry);
            dedup_keys.insert(dedup_key.clone(), job_id.clone());
        }

        tracing::debug!(
            job_id = %job_id,
            dedup_key = %dedup_key,
            "Registered job with pre-generated ID"
        );

        // Notify monitors of new job (this triggers auto-dispatch)
        self.monitors.broadcast_created(&job_info).await;

        Ok(true)
    }
}
