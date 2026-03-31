//! Background job system initialization and management
//!
//! This module handles initialization of the unified job system, including:
//! - Job handler registry setup
//! - Worker pool creation and startup
//! - Event handler subscription
//! - Job restoration after crash/restart
//! - Watchdog and cleanup tasks

mod flow_events;
mod init_system;
mod restore;

use super::RocksDBStorage;
use raisin_error::Result;

impl RocksDBStorage {
    /// Queue a background job to build property index for a tenant/repo/branch/workspace
    ///
    /// This method creates a PropertyIndexBuild job and queues it in the job system.
    /// The job will be processed asynchronously by the worker pool.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace` - Workspace identifier
    ///
    /// # Returns
    ///
    /// Returns the JobId for tracking the job status
    pub async fn queue_property_index_build(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
    ) -> Result<raisin_storage::jobs::JobId> {
        use raisin_hlc::HLC;
        use raisin_storage::jobs::{JobContext, JobType};
        use std::collections::HashMap;

        // Create job context
        let context = JobContext {
            tenant_id: tenant_id.to_string(),
            repo_id: repo_id.to_string(),
            branch: branch.to_string(),
            workspace_id: workspace.to_string(),
            revision: HLC::new(0, 0), // Not applicable for index build
            metadata: HashMap::new(),
        };

        // Register job
        let job_id = self
            .job_registry
            .register_job(
                JobType::PropertyIndexBuild {
                    tenant_id: tenant_id.to_string(),
                    repo_id: repo_id.to_string(),
                    branch: branch.to_string(),
                    workspace: workspace.to_string(),
                },
                Some(tenant_id.to_string()),
                None,
                None,
                None,
            )
            .await?;

        // Store job context
        self.job_data_store.put(&job_id, &context)?;

        tracing::info!(
            job_id = %job_id,
            tenant = %tenant_id,
            repo = %repo_id,
            branch = %branch,
            workspace = %workspace,
            "Queued property index build job"
        );

        Ok(job_id)
    }

    /// Get master encryption key from environment variable
    ///
    /// Reads the `RAISIN_MASTER_KEY` environment variable and converts it to a 32-byte key.
    /// The key must be exactly 64 hexadecimal characters (32 bytes).
    ///
    /// # Returns
    ///
    /// A 32-byte array containing the master encryption key
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - `RAISIN_MASTER_KEY` environment variable is not set
    /// - The key is not valid hexadecimal
    /// - The key is not exactly 32 bytes
    fn get_master_encryption_key() -> Result<[u8; 32]> {
        let key_hex = std::env::var("RAISIN_MASTER_KEY").map_err(|_| {
            raisin_error::Error::Validation(
                "RAISIN_MASTER_KEY environment variable not set".to_string(),
            )
        })?;

        let key_bytes = hex::decode(&key_hex).map_err(|e| {
            raisin_error::Error::Validation(format!(
                "Invalid RAISIN_MASTER_KEY: not valid hex: {}",
                e
            ))
        })?;

        if key_bytes.len() != 32 {
            return Err(raisin_error::Error::Validation(format!(
                "Invalid RAISIN_MASTER_KEY: expected 32 bytes, got {}",
                key_bytes.len()
            )));
        }

        let mut key = [0u8; 32];
        key.copy_from_slice(&key_bytes);

        Ok(key)
    }
}
