//! Management operations for in-memory storage
//!
//! Provides simple implementations of management operations for testing
//! and development use.

use crate::InMemoryStorage;
use raisin_error::Result;
use raisin_storage::{
    jobs::{global_registry, JobType},
    BackgroundJobs, JobHandle, JobId,
};
use std::time::Duration;

impl BackgroundJobs for InMemoryStorage {
    fn start_background_jobs(&self) -> Result<JobHandle> {
        // In-memory storage doesn't need background jobs
        // Return disabled for simplicity
        Ok(JobHandle::Disabled)
    }

    fn schedule_integrity_scan(&self, tenant: &str, _interval: Duration) -> Result<JobId> {
        use futures::executor::block_on;

        // Register a simple job that completes immediately
        let tenant = tenant.to_string();

        // Register the job
        let job_id = block_on(async {
            let job_id = global_registry()
                .register_job(
                    JobType::IntegrityScan,
                    Some(tenant.clone()),
                    None,
                    None,
                    None,
                )
                .await?;

            // Mark as running
            global_registry().mark_running(&job_id).await?;

            // Immediately mark as completed since in-memory has no real work
            global_registry().mark_completed(&job_id).await?;

            Ok::<JobId, raisin_error::Error>(job_id)
        })?;

        Ok(job_id)
    }

    // The async methods get_job_status, cancel_job, list_jobs, and wait_for_job
    // are provided by the trait's default implementations using the global registry
}
