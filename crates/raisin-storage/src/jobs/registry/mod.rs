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
        }
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
                Some("test_tenant".to_string()),
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
        assert_eq!(info.tenant, Some("test_tenant".to_string()));
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
                None,
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
            .register_job(JobType::IntegrityScan, None, None, None, None)
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
}
