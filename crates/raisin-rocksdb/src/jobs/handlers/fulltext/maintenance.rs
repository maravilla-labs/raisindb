// SPDX-License-Identifier: BSL-1.1

//! Operator-triggered fulltext index maintenance: verify, rebuild,
//! reconcile, optimize, purge.
//!
//! These used to run in a detached `tokio::spawn` from the HTTP layer,
//! which registered a job and never wrote its `JobContext`. The worker
//! claimed the contextless job and marked it `Failed: Job context not
//! found`, clobbering the status the detached task was writing — so an
//! operator running `/fulltext/optimize` during a merge storm was told
//! it had failed while it was in fact merging. Running the work HERE,
//! under the worker that already owns the job, is what makes the status
//! field true: the worker marks Executing, stores the handler's return
//! value as the job result, and marks Completed or Failed from the one
//! outcome.
//!
//! The transport writes the context first and registers under that id
//! (`register_job_with_id`), so dispatch can never observe the job
//! without its context.

use raisin_error::Result;
use raisin_indexer::TantivyManagement;
use raisin_storage::jobs::{JobContext, JobInfo, JobType};

use super::handler::FulltextJobHandler;

/// Metadata key distinguishing the two operations that share
/// `JobType::FulltextRebuild`. Absent (or anything else) means a full
/// rebuild; `"reconcile"` means the non-destructive replay.
pub const META_FULLTEXT_MODE: &str = "fulltext_mode";
/// Value of [`META_FULLTEXT_MODE`] selecting the reconcile path.
pub const FULLTEXT_MODE_RECONCILE: &str = "reconcile";

impl FulltextJobHandler {
    /// Build a management facade over the engine this handler already
    /// owns. `TantivyManagement` is a `PathBuf` plus an `Arc` clone, so
    /// constructing it per job is free and avoids threading a 36th
    /// argument through the handler registry's constructor.
    fn management(&self) -> TantivyManagement {
        TantivyManagement::new(
            self.tantivy_engine.base_path().to_path_buf(),
            self.tantivy_engine.clone(),
        )
    }

    /// Handle the four operator-triggered maintenance job types.
    ///
    /// Returns the operation's report/stats as the job result, matching
    /// what the admin console used to read off the detached task.
    /// Errors are returned raw so the worker owns the failure — these
    /// jobs are registered with `max_retries = 0`, so an error is
    /// terminal and reported once.
    pub async fn handle_maintenance(
        &self,
        job: &JobInfo,
        context: &JobContext,
    ) -> Result<Option<serde_json::Value>> {
        let tenant = &context.tenant_id;
        let repo = &context.repo_id;
        let branch = &context.branch;

        match &job.job_type {
            JobType::FulltextVerify => {
                tracing::info!(
                    tenant, repo, branch,
                    job_id = %job.id,
                    "Fulltext verification starting"
                );
                let report = self.management().verify_index(tenant, repo, branch).await?;
                tracing::info!(
                    tenant, repo, branch,
                    status = ?report.status,
                    "Fulltext verification completed"
                );
                Ok(Some(serde_json::to_value(&report).unwrap_or_default()))
            }

            JobType::FulltextRebuild => {
                let reconcile = context
                    .metadata
                    .get(META_FULLTEXT_MODE)
                    .and_then(|v| v.as_str())
                    == Some(FULLTEXT_MODE_RECONCILE);

                let stats = if reconcile {
                    tracing::info!(
                        tenant, repo, branch,
                        job_id = %job.id,
                        "Fulltext reconcile starting"
                    );
                    crate::management::reconcile_fulltext_index(
                        &self.storage,
                        &self.tantivy_engine,
                        tenant,
                        repo,
                        branch,
                    )
                    .await?
                } else {
                    tracing::info!(
                        tenant, repo, branch,
                        job_id = %job.id,
                        "Fulltext rebuild starting"
                    );
                    crate::management::rebuild_fulltext_index(
                        &self.storage,
                        &self.tantivy_engine,
                        tenant,
                        repo,
                        branch,
                    )
                    .await?
                };

                tracing::info!(
                    tenant,
                    repo,
                    branch,
                    reconcile,
                    items_processed = stats.items_processed,
                    "Fulltext rebuild/reconcile completed"
                );
                Ok(Some(serde_json::to_value(&stats).unwrap_or_default()))
            }

            JobType::FulltextOptimize => {
                tracing::info!(
                    tenant, repo, branch,
                    job_id = %job.id,
                    "Fulltext optimization starting"
                );
                let stats = self
                    .management()
                    .optimize_index(tenant, repo, branch)
                    .await?;
                tracing::info!(
                    tenant,
                    repo,
                    branch,
                    segments_merged = stats.segments_merged,
                    "Fulltext optimization completed"
                );
                Ok(Some(serde_json::to_value(&stats).unwrap_or_default()))
            }

            JobType::FulltextPurge => {
                tracing::warn!(
                    tenant, repo, branch,
                    job_id = %job.id,
                    "Fulltext purge starting"
                );
                self.management().purge_index(tenant, repo, branch).await?;
                tracing::info!(tenant, repo, branch, "Fulltext purge completed");
                Ok(None)
            }

            other => Err(raisin_error::Error::Validation(format!(
                "Not a fulltext maintenance job type: {}",
                other
            ))),
        }
    }
}
