//! Property index building job handler
//!
//! This module handles background property index building operations
//! for lazy indexing during cluster replication catch-up scenarios.

use raisin_error::{Error, Result};
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use std::sync::Arc;

use crate::lazy_indexing::LazyIndexManager;

/// Handler for property index building jobs
///
/// This handler processes PropertyIndexBuild jobs by:
/// 1. Extracting tenant/repo/branch/workspace parameters from JobType
/// 2. Scanning all nodes in the scope
/// 3. Building PROPERTY_INDEX entries for all node properties
/// 4. Updating INDEX_STATUS_CF to track build completion
pub struct PropertyIndexJobHandler {
    lazy_index_manager: Arc<LazyIndexManager>,
}

impl PropertyIndexJobHandler {
    /// Create a new property index job handler
    ///
    /// # Arguments
    ///
    /// * `lazy_index_manager` - Lazy indexing manager for building property indexes
    pub fn new(lazy_index_manager: Arc<LazyIndexManager>) -> Self {
        Self { lazy_index_manager }
    }

    /// Handle property index build job
    ///
    /// Processes a PropertyIndexBuild job variant which builds the property
    /// index for all nodes in a tenant/repo/branch/workspace scope.
    ///
    /// # Arguments
    ///
    /// * `job` - Job information containing the JobType::PropertyIndexBuild variant
    /// * `context` - Job context with tenant, repo, branch, workspace info
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Job type is not PropertyIndexBuild
    /// - Property index building fails
    /// - INDEX_STATUS_CF update fails
    pub async fn handle(&self, job: &JobInfo, context: &JobContext) -> Result<()> {
        // Extract parameters from JobType
        let (tenant_id, repo_id, branch, workspace) = match &job.job_type {
            JobType::PropertyIndexBuild {
                tenant_id,
                repo_id,
                branch,
                workspace,
            } => (
                tenant_id.as_str(),
                repo_id.as_str(),
                branch.as_str(),
                workspace.as_str(),
            ),
            _ => {
                return Err(Error::Validation(
                    "Expected PropertyIndexBuild job type".to_string(),
                ))
            }
        };

        tracing::info!(
            job_id = %job.id,
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            branch = %branch,
            workspace = %workspace,
            "Processing property index build job"
        );

        // Build property index for this scope
        let result = self
            .lazy_index_manager
            .build_property_index(tenant_id, repo_id, branch, workspace)
            .await?;

        tracing::info!(
            job_id = %job.id,
            nodes_processed = result.nodes_processed,
            properties_indexed = result.properties_indexed,
            elapsed_ms = result.elapsed.as_millis(),
            "Property index build completed"
        );

        // Mark index status with the job's revision
        self.lazy_index_manager.set_property_index_status(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &context.revision,
        )?;

        tracing::debug!(
            job_id = %job.id,
            revision = %context.revision,
            "Updated INDEX_STATUS_CF with build completion"
        );

        Ok(())
    }
}
