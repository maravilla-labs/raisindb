//! Enqueueing a local spatial index build.
//!
//! The body lives in a free function rather than a `RocksDBStorage` method so that
//! the admin surface ([`crate::spatial_state::admin::SpatialAdminStore`]) can reuse
//! it. Two copies of "register a job" is precisely the drift that left the
//! replication path without spatial indexing in the first place.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_storage::jobs::{JobContext, JobId, JobRegistry, JobType};

use crate::jobs::JobDataStore;
use crate::spatial_state::admin::BuildEnqueuer;

/// Register a `JobType::SpatialIndexBuild` for one workspace (optionally one
/// property).
///
/// # Revision
///
/// Wall-clock now, and it is used ONLY for the state record's `built_through`
/// watermark. The handler stamps each index ENTRY at the revision of the node
/// record it describes, never at this one — a rebuild does not advance the branch
/// head, so an entry stamped "now" is in the future relative to every read and is
/// discarded by the MVCC filter. See `jobs::handlers::spatial_index::build`.
pub(crate) async fn enqueue_spatial_index_build(
    job_registry: &JobRegistry,
    job_data_store: &JobDataStore,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    property: Option<&str>,
    rebuild: bool,
) -> Result<JobId> {
    let revision = HLC::new(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        0,
    );

    let context = JobContext {
        tenant_id: tenant_id.to_string(),
        repo_id: repo_id.to_string(),
        branch: branch.to_string(),
        workspace_id: workspace.to_string(),
        revision,
        metadata: HashMap::new(),
    };

    // Context before registration, so dispatch can never observe a job without
    // its context.
    let job_id = JobId::new();
    job_data_store.put(&job_id, &context)?;

    job_registry
        .register_job_with_id(
            job_id.clone(),
            JobType::SpatialIndexBuild {
                tenant_id: tenant_id.to_string(),
                repo_id: repo_id.to_string(),
                branch: branch.to_string(),
                workspace: workspace.to_string(),
                property: property.map(|p| p.to_string()),
                rebuild,
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
        property = ?property,
        rebuild,
        "Queued spatial index build job"
    );

    Ok(job_id)
}

/// The job-system-backed enqueuer handed to the admin surface.
pub struct JobSystemEnqueuer {
    job_registry: Arc<JobRegistry>,
    job_data_store: Arc<JobDataStore>,
}

impl JobSystemEnqueuer {
    pub fn new(job_registry: Arc<JobRegistry>, job_data_store: Arc<JobDataStore>) -> Self {
        Self {
            job_registry,
            job_data_store,
        }
    }
}

#[async_trait]
impl BuildEnqueuer for JobSystemEnqueuer {
    async fn enqueue(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property: Option<&str>,
        rebuild: bool,
    ) -> Result<String> {
        enqueue_spatial_index_build(
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
        .map(|id| id.0)
    }
}
