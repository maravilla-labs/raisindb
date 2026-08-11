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
pub(crate) mod spatial;

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

        // Store job context BEFORE registering so dispatch can never
        // observe the job without its context.
        let job_id = raisin_storage::jobs::JobId::new();
        self.job_data_store.put(&job_id, &context)?;

        // Register job under the pre-generated ID
        self.job_registry
            .register_job_with_id(
                job_id.clone(),
                JobType::PropertyIndexBuild {
                    tenant_id: tenant_id.to_string(),
                    repo_id: repo_id.to_string(),
                    branch: branch.to_string(),
                    workspace: workspace.to_string(),
                },
                tenant_id.to_string(),
                None,
                None,
                None,
            )
            .await?;

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

    /// Queue a spatial index build (or rebuild) for a workspace.
    ///
    /// `property = None` covers every geometry-valued property in the workspace.
    /// `rebuild = true` re-emits every entry and tombstones superseded ones;
    /// `false` fills gaps only.
    ///
    /// # Idempotency
    ///
    /// `JobType::SpatialIndexBuild`'s `dedup_key` is
    /// `spatial:{tenant}:{repo}:{branch}:{ws}:{property|*}`, so a duplicate request
    /// while one is queued or running collapses onto the existing job — the same
    /// mechanism the fulltext path uses.
    ///
    /// # Scope
    ///
    /// **LOCAL to this node.** The spatial index is derived local state, so a repair
    /// must be run on each node (or via each node's HTTP endpoint). Cluster-wide
    /// fan-out happens through *configuration*: `WorkspaceConfig.spatial` is
    /// replicated, so a policy change reaches every peer, and each peer then observes
    /// the `policy_hash` mismatch and schedules its own build.
    pub async fn queue_spatial_index_build(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property: Option<&str>,
        rebuild: bool,
    ) -> Result<raisin_storage::jobs::JobId> {
        spatial::enqueue_spatial_index_build(
            &self.job_registry,
            &self.job_data_store,
            tenant_id,
            repo_id,
            branch,
            workspace,
            property,
            rebuild,
        )
        .await
    }

    /// Get the master encryption key.
    ///
    /// Delegates to the shared `raisin-crypto` loader, which reads
    /// `RAISIN_MASTER_KEY` with the legacy `EMBEDDING_MASTER_KEY` fallback.
    ///
    /// # Errors
    ///
    /// Returns an error if neither variable is set, or if the value present is
    /// not 32 bytes of hex.
    fn get_master_encryption_key() -> Result<[u8; 32]> {
        raisin_crypto::master_key_with_embedding_fallback()?.ok_or_else(|| {
            raisin_error::Error::Validation(
                "RAISIN_MASTER_KEY environment variable not set".to_string(),
            )
        })
    }
}
