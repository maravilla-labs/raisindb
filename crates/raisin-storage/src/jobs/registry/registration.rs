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
    /// * `tenant` - Tenant identifier (required)
    /// * `handle` - Optional task handle for the job
    /// * `cancel_token` - Optional cancellation token
    /// * `max_retries` - Optional maximum retry attempts (default: 3, use 0 for no retries)
    pub async fn register_job(
        &self,
        job_type: JobType,
        tenant: String,
        handle: Option<JoinHandle<()>>,
        cancel_token: Option<Arc<tokio_util::sync::CancellationToken>>,
        max_retries: Option<u32>,
    ) -> Result<JobId> {
        self.register_job_inner(
            JobId::new(),
            job_type,
            tenant,
            handle,
            cancel_token,
            max_retries,
            None,
        )
        .await
    }

    /// Register a new job under a caller-generated job ID.
    ///
    /// This is the race-free variant of [`register_job`](Self::register_job):
    /// generate a `JobId` with `JobId::new()`, persist the job's `JobContext`
    /// (e.g. via `JobDataStore::put`) FIRST, and only then call this method.
    ///
    /// `register_job` generates the ID internally, which forces callers to
    /// write the job context AFTER registration. Registration immediately
    /// broadcasts the job to dispatch monitors, so a worker can claim the job
    /// before the context write lands and fail with "Missing job context".
    /// Storing the context before calling this method closes that window.
    pub async fn register_job_with_id(
        &self,
        job_id: JobId,
        job_type: JobType,
        tenant: String,
        handle: Option<JoinHandle<()>>,
        cancel_token: Option<Arc<tokio_util::sync::CancellationToken>>,
        max_retries: Option<u32>,
    ) -> Result<JobId> {
        self.register_job_inner(
            job_id,
            job_type,
            tenant,
            handle,
            cancel_token,
            max_retries,
            None,
        )
        .await
    }

    /// Register a new job to be dispatched at a future time
    ///
    /// The job is persisted immediately but only dispatched to workers once
    /// `scheduled_at` is reached (via the dispatching monitor's delayed
    /// dispatch; restart-safe because `next_retry_at` is persisted and
    /// honored when restored jobs are re-dispatched).
    ///
    /// Used for flow wait timeouts, retry backoffs, and scheduled delays.
    pub async fn register_job_at(
        &self,
        job_type: JobType,
        tenant: String,
        scheduled_at: chrono::DateTime<Utc>,
        max_retries: Option<u32>,
    ) -> Result<JobId> {
        self.register_job_inner(
            JobId::new(),
            job_type,
            tenant,
            None,
            None,
            max_retries,
            Some(scheduled_at),
        )
        .await
    }

    /// Register a delayed job under a caller-generated job ID.
    ///
    /// Like [`register_job_at`](Self::register_job_at), but lets the caller
    /// store the `JobContext` BEFORE the job becomes visible to dispatch.
    /// See [`register_job_with_id`](Self::register_job_with_id) for why.
    pub async fn register_job_at_with_id(
        &self,
        job_id: JobId,
        job_type: JobType,
        tenant: String,
        scheduled_at: chrono::DateTime<Utc>,
        max_retries: Option<u32>,
    ) -> Result<JobId> {
        self.register_job_inner(
            job_id,
            job_type,
            tenant,
            None,
            None,
            max_retries,
            Some(scheduled_at),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn register_job_inner(
        &self,
        job_id: JobId,
        job_type: JobType,
        tenant: String,
        handle: Option<JoinHandle<()>>,
        cancel_token: Option<Arc<tokio_util::sync::CancellationToken>>,
        max_retries: Option<u32>,
        scheduled_at: Option<chrono::DateTime<Utc>>,
    ) -> Result<JobId> {
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
            // None = process immediately; Some(future) = delayed dispatch
            next_retry_at: scheduled_at,
            executing_since: None,
        };

        let job_info = entry.to_job_info();

        // CRITICAL: Do not hold any registry lock across an await point.
        // broadcast_created() is async and can fan out to monitors that call
        // back into the registry (dispatch, persistence). Holding the jobs
        // write-lock across that await deadlocked the entire commit path.
        {
            let mut jobs = self.jobs.write().await;
            jobs.insert(job_id.clone(), entry);
        }

        if let Some(h) = handle {
            let mut handles = self.handles.write().await;
            handles.insert(job_id.clone(), h);
        }

        // Persist delayed jobs at registration so they survive a restart
        // while still pending. Immediately-dispatched jobs are persisted on
        // their first status transition (claim), but a job scheduled for the
        // future (register_job_at) has no status change until it fires — a
        // restart in between would otherwise silently drop it.
        if job_info.next_retry_at.is_some() {
            if let Some(persistence) = &self.persistence {
                if let Err(e) = persistence.persist_job(&job_id, &job_info).await {
                    tracing::error!(
                        job_id = %job_id,
                        error = %e,
                        "Failed to persist delayed job at registration"
                    );
                }
            }
        }

        // Notify monitors of new job (no locks held)
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
    /// * `tenant` - Tenant identifier (required)
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
        tenant: String,
        dedup_key: String,
        max_retries: Option<u32>,
    ) -> Result<Option<JobId>> {
        // Check if an active job with this dedup key already exists.
        //
        // LOCK ORDER INVARIANT: `jobs` before `dedup_keys`, never the reverse.
        // The insert block below nests `jobs.write()` → `dedup_keys.write()`;
        // nesting `dedup_keys.read()` → `jobs.read()` here (as this code once
        // did) is the opposite order — two concurrent registrations racing
        // through the two blocks ABBA-deadlock the registry, and because the
        // write-preferring `jobs` lock backs every heartbeat/claim/status
        // update, that froze the ENTIRE job system under bursty load. Copy the
        // id out and drop the dedup guard before touching `jobs`.
        let existing_job_id = {
            let dedup_keys = self.dedup_keys.read().await;
            dedup_keys.get(&dedup_key).cloned()
        };
        if let Some(existing_job_id) = existing_job_id {
            // Check if the existing job is still active (dedup guard dropped)
            let jobs = self.jobs.read().await;
            if let Some(job) = jobs.get(&existing_job_id) {
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
            executing_since: None,
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
    /// * `tenant` - Tenant identifier (required)
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
        tenant: String,
        dedup_key: String,
        max_retries: Option<u32>,
    ) -> Result<bool> {
        // Check if an active job with this dedup key already exists.
        // LOCK ORDER INVARIANT: `jobs` before `dedup_keys` — see
        // register_job_idempotent for the ABBA-deadlock rationale.
        let existing_job_id = {
            let dedup_keys = self.dedup_keys.read().await;
            dedup_keys.get(&dedup_key).cloned()
        };
        if let Some(existing_job_id) = existing_job_id {
            let jobs = self.jobs.read().await;
            if let Some(job) = jobs.get(&existing_job_id) {
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
            executing_since: None,
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
