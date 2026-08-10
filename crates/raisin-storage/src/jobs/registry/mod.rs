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

//! Shared job registry for tracking background jobs across all storage implementations
//!
//! This provides a centralized way to track job status, cancel jobs, and manage
//! background tasks regardless of the storage backend being used.

mod job_entry;
mod lifecycle;
mod queries;
mod registration;
mod status;

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use crate::jobs::{JobId, JobMonitorHub};

/// Shared job registry for tracking background jobs
#[derive(Clone)]
pub struct JobRegistry {
    jobs: Arc<RwLock<HashMap<JobId, job_entry::JobEntry>>>,
    handles: Arc<RwLock<HashMap<JobId, JoinHandle<()>>>>,
    monitors: Arc<JobMonitorHub>,
    persistence: Option<Arc<dyn crate::jobs::JobPersistence>>,
    /// Deduplication keys mapped to job IDs for idempotent job registration
    dedup_keys: Arc<RwLock<HashMap<String, JobId>>>,
    /// Count of non-terminal (Scheduled/Running/Executing) jobs per tenant.
    /// Incremented in `register_job_inner`, decremented in `update_status`
    /// on the (exactly-once, guarded by the existing terminal-transition
    /// check) transition into a terminal status. This is the physical
    /// backstop behind `max_active_jobs_per_tenant`: registration —
    /// not dispatch-queue capacity — is what a runaway trigger/function
    /// loop actually grows without bound (dispatch just logs a warning
    /// and leaves the already-registered job in place when its queue is
    /// full), so the cap has to be enforced here.
    tenant_active_counts: Arc<RwLock<HashMap<String, usize>>>,
    /// Maximum non-terminal jobs one tenant may have registered at once.
    /// `None` disables the check (default — opt-in via
    /// `with_tenant_job_cap`).
    max_active_jobs_per_tenant: Option<usize>,
}

impl Default for JobRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl JobRegistry {
    /// Create a new job registry
    pub fn new() -> Self {
        Self {
            jobs: Arc::new(RwLock::new(HashMap::new())),
            handles: Arc::new(RwLock::new(HashMap::new())),
            monitors: Arc::new(JobMonitorHub::new()),
            persistence: None,
            dedup_keys: Arc::new(RwLock::new(HashMap::new())),
            tenant_active_counts: Arc::new(RwLock::new(HashMap::new())),
            max_active_jobs_per_tenant: None,
        }
    }

    /// Cap the number of non-terminal (Scheduled/Running/Executing) jobs a
    /// single tenant may have registered at once. Once a tenant is at the
    /// cap, further registrations fail with a validation error until some
    /// of its jobs complete/fail/are cancelled. `None` (the default)
    /// disables the check entirely.
    pub fn with_tenant_job_cap(mut self, max_active_jobs_per_tenant: Option<usize>) -> Self {
        self.max_active_jobs_per_tenant = max_active_jobs_per_tenant;
        self
    }

    /// Current count of non-terminal jobs for a tenant.
    pub async fn active_job_count(&self, tenant: &str) -> usize {
        self.tenant_active_counts
            .read()
            .await
            .get(tenant)
            .copied()
            .unwrap_or(0)
    }

    /// Set the persistence backend for this registry
    ///
    /// This enables automatic persistence of job state changes to durable storage,
    /// enabling crash recovery and job history tracking.
    ///
    /// # Arguments
    ///
    /// * `persistence` - Implementation of the JobPersistence trait
    ///
    /// # Returns
    ///
    /// Self for method chaining (builder pattern)
    pub fn with_persistence(mut self, persistence: Arc<dyn crate::jobs::JobPersistence>) -> Self {
        self.persistence = Some(persistence);
        self
    }

    /// Get the monitor hub for registering job monitors
    pub fn monitors(&self) -> &Arc<JobMonitorHub> {
        &self.monitors
    }

    /// Number of job entries currently held in memory.
    ///
    /// Surfaced for the memory diagnostics endpoint. Note there are TWO
    /// registries in a running server — the storage-owned one, which the 60s
    /// cleanup sweep trims, and the process-wide [`global_registry`], which
    /// nothing sweeps. Admin-triggered work (integrity scans, backups,
    /// maintenance, vector reindexes) registers against the latter, so its
    /// count only ever goes up. Sample both.
    pub async fn job_count(&self) -> usize {
        self.jobs.read().await.len()
    }
}

/// Global job registry instance
static GLOBAL_REGISTRY: once_cell::sync::Lazy<JobRegistry> =
    once_cell::sync::Lazy::new(JobRegistry::new);

/// Get the global job registry
pub fn global_registry() -> &'static JobRegistry {
    &GLOBAL_REGISTRY
}

impl fmt::Debug for JobRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JobRegistry")
            .field("job_count", &"<async>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{JobStatus, JobType};

    #[tokio::test]
    async fn test_job_lifecycle() {
        let registry = JobRegistry::new();

        // Register a job
        let job_id = registry
            .register_job(
                JobType::IntegrityScan,
                "test_tenant".to_string(),
                None,
                None,
                None,
            )
            .await
            .unwrap();

        // Check initial status
        assert_eq!(
            registry.get_status(&job_id).await.unwrap(),
            JobStatus::Scheduled
        );

        // Mark as running
        registry.mark_running(&job_id).await.unwrap();
        assert_eq!(
            registry.get_status(&job_id).await.unwrap(),
            JobStatus::Running
        );

        // Update progress
        registry.update_progress(&job_id, 0.5).await.unwrap();

        // Mark as completed
        registry.mark_completed(&job_id).await.unwrap();
        assert_eq!(
            registry.get_status(&job_id).await.unwrap(),
            JobStatus::Completed
        );

        // Check job info
        let info = registry.get_job_info(&job_id).await.unwrap();
        assert_eq!(info.job_type, JobType::IntegrityScan);
        assert_eq!(info.tenant, "test_tenant".to_string());
        assert!(info.completed_at.is_some());
    }

    #[tokio::test]
    async fn test_job_cancellation() {
        let registry = JobRegistry::new();

        // Register a job with cancellation token
        let cancel_token = Arc::new(tokio_util::sync::CancellationToken::new());
        let job_id = registry
            .register_job(
                JobType::Backup,
                "test_tenant".to_string(),
                None,
                Some(cancel_token.clone()),
                None,
            )
            .await
            .unwrap();

        // Cancel the job
        registry.cancel_job(&job_id).await.unwrap();

        // Check that token was cancelled
        assert!(cancel_token.is_cancelled());

        // Check status
        assert_eq!(
            registry.get_status(&job_id).await.unwrap(),
            JobStatus::Cancelled
        );
    }

    #[tokio::test]
    async fn test_terminal_status_is_immutable() {
        let registry = JobRegistry::new();
        let job_id = registry
            .register_job(
                JobType::IntegrityScan,
                "test_tenant".to_string(),
                None,
                None,
                None,
            )
            .await
            .unwrap();

        registry
            .mark_failed(&job_id, "[timeout_final] timeout".to_string())
            .await
            .unwrap();

        let err = registry.mark_completed(&job_id).await.unwrap_err();
        assert!(format!("{}", err).contains("Cannot transition terminal job"));

        assert!(matches!(
            registry.get_status(&job_id).await.unwrap(),
            JobStatus::Failed(_)
        ));
    }

    #[tokio::test]
    async fn tenant_job_cap_rejects_once_at_the_limit() {
        let registry = JobRegistry::new().with_tenant_job_cap(Some(2));

        registry
            .register_job(JobType::IntegrityScan, "t".to_string(), None, None, None)
            .await
            .unwrap();
        registry
            .register_job(JobType::IntegrityScan, "t".to_string(), None, None, None)
            .await
            .unwrap();

        let err = registry
            .register_job(JobType::IntegrityScan, "t".to_string(), None, None, None)
            .await
            .unwrap_err();
        assert!(format!("{}", err).contains("at the configured cap"));

        // A different tenant is unaffected.
        registry
            .register_job(
                JobType::IntegrityScan,
                "other".to_string(),
                None,
                None,
                None,
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn tenant_job_cap_frees_up_after_a_job_completes() {
        let registry = JobRegistry::new().with_tenant_job_cap(Some(1));

        let job_id = registry
            .register_job(JobType::IntegrityScan, "t".to_string(), None, None, None)
            .await
            .unwrap();
        assert_eq!(registry.active_job_count("t").await, 1);

        registry
            .register_job(JobType::IntegrityScan, "t".to_string(), None, None, None)
            .await
            .unwrap_err();

        registry.mark_completed(&job_id).await.unwrap();
        assert_eq!(registry.active_job_count("t").await, 0);

        registry
            .register_job(JobType::IntegrityScan, "t".to_string(), None, None, None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn capacity_eviction_removes_oldest_terminal_only() {
        let registry = JobRegistry::new();

        // Two terminal jobs (completed in registration order) + one active.
        let old_terminal = registry
            .register_job(JobType::IntegrityScan, "t".to_string(), None, None, None)
            .await
            .unwrap();
        registry.mark_completed(&old_terminal).await.unwrap();

        let new_terminal = registry
            .register_job(JobType::IntegrityScan, "t".to_string(), None, None, None)
            .await
            .unwrap();
        registry.mark_completed(&new_terminal).await.unwrap();

        let active = registry
            .register_job(JobType::IntegrityScan, "t".to_string(), None, None, None)
            .await
            .unwrap();

        // Cap of 2 → evict exactly the oldest terminal entry.
        let evicted = registry.enforce_registry_capacity(2).await;
        assert_eq!(evicted, 1);
        assert!(registry.get_status(&old_terminal).await.is_err());
        assert!(registry.get_status(&new_terminal).await.is_ok());
        assert!(registry.get_status(&active).await.is_ok());

        // Cap of 0 with only active jobs left over the cap → active survives.
        let evicted = registry.enforce_registry_capacity(0).await;
        assert_eq!(evicted, 1); // the remaining terminal entry
        assert!(registry.get_status(&active).await.is_ok());

        // Under cap → no-op.
        assert_eq!(registry.enforce_registry_capacity(10).await, 0);
    }

    #[tokio::test]
    async fn trim_drops_results_only_from_aged_terminal_jobs() {
        let registry = JobRegistry::new();

        let done = registry
            .register_job(JobType::IntegrityScan, "t".to_string(), None, None, None)
            .await
            .unwrap();
        registry
            .set_result(&done, serde_json::json!({"big": "payload"}))
            .await
            .unwrap();
        registry.mark_completed(&done).await.unwrap();

        let active = registry
            .register_job(JobType::IntegrityScan, "t".to_string(), None, None, None)
            .await
            .unwrap();
        registry
            .set_result(&active, serde_json::json!({"partial": true}))
            .await
            .unwrap();

        // Inside the grace window: nothing is trimmed.
        assert_eq!(
            registry
                .trim_terminal_results(chrono::Duration::minutes(10))
                .await,
            0
        );
        assert!(registry.get_job_info(&done).await.unwrap().result.is_some());

        // Past the grace window (negative grace puts the cutoff in the
        // future): only the terminal job's result is dropped.
        assert_eq!(
            registry
                .trim_terminal_results(chrono::Duration::seconds(-1))
                .await,
            1
        );
        assert!(registry.get_job_info(&done).await.unwrap().result.is_none());
        assert!(registry
            .get_job_info(&active)
            .await
            .unwrap()
            .result
            .is_some());
    }

    #[tokio::test]
    async fn restore_counts_non_terminal_jobs_against_tenant_cap() {
        use crate::jobs::JobInfo;
        use chrono::Utc;

        let registry = JobRegistry::new().with_tenant_job_cap(Some(1));

        let restored = JobInfo {
            id: crate::jobs::JobId::new(),
            job_type: JobType::IntegrityScan,
            status: JobStatus::Scheduled,
            tenant: "t".to_string(),
            started_at: Utc::now(),
            completed_at: None,
            progress: None,
            error: None,
            result: None,
            retry_count: 0,
            max_retries: 3,
            last_heartbeat: None,
            timeout_seconds: 300,
            next_retry_at: None,
            executing_since: None,
        };
        let restored_id = restored.id.clone();
        registry.restore_job(restored).await.unwrap();
        assert_eq!(registry.active_job_count("t").await, 1);

        // Cap is 1 → a fresh registration must be refused.
        registry
            .register_job(JobType::IntegrityScan, "t".to_string(), None, None, None)
            .await
            .unwrap_err();

        // Terminal transition frees the slot exactly once.
        registry.mark_completed(&restored_id).await.unwrap();
        assert_eq!(registry.active_job_count("t").await, 0);
    }

    #[tokio::test]
    async fn tenant_job_cap_disabled_by_default() {
        let registry = JobRegistry::new();
        for _ in 0..10 {
            registry
                .register_job(JobType::IntegrityScan, "t".to_string(), None, None, None)
                .await
                .unwrap();
        }
        assert_eq!(registry.active_job_count("t").await, 10);
    }
}
