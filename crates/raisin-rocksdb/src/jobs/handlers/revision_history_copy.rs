//! Revision history copy job handler
//!
//! This handler copies revision metadata (commit history) from a source branch
//! to a target branch during branch creation. It processes revisions in batches
//! to avoid memory pressure with large histories.

use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use raisin_storage::RevisionMeta;
use rocksdb::DB;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Batch size for processing revisions
const BATCH_SIZE: usize = 1000;

/// Handler for revision history copy jobs
pub struct RevisionHistoryCopyHandler {
    db: Arc<DB>,
}

impl RevisionHistoryCopyHandler {
    /// Create a new revision history copy handler
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Handle a revision history copy job
    pub async fn handle(&self, job: &JobInfo, context: &JobContext) -> Result<()> {
        // Extract job parameters
        let (source_branch, target_branch, up_to_revision) = match &job.job_type {
            JobType::RevisionHistoryCopy {
                source_branch,
                target_branch,
                up_to_revision,
            } => (source_branch, target_branch, up_to_revision),
            _ => {
                return Err(raisin_error::Error::Validation(format!(
                    "Invalid job type for RevisionHistoryCopyHandler: {:?}",
                    job.job_type
                )));
            }
        };

        let tenant_id = &context.tenant_id;
        let repo_id = &context.repo_id;

        info!(
            job_id = %job.id,
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            source_branch = %source_branch,
            target_branch = %target_branch,
            up_to_revision = %up_to_revision,
            "Starting revision history copy"
        );

        // List and copy revisions in batches
        let mut total_copied = 0;
        let mut offset = 0;

        loop {
            let revisions = self
                .list_revisions_for_branch(tenant_id, repo_id, source_branch, BATCH_SIZE, offset)
                .await?;

            if revisions.is_empty() {
                break;
            }

            let batch_size = revisions.len();

            for revision_meta in revisions {
                // Only copy revisions up to the specified max revision
                if revision_meta.revision > *up_to_revision {
                    continue;
                }

                // Skip if this revision is not from the source branch
                if revision_meta.branch != *source_branch {
                    continue;
                }

                // Create a new revision meta with the target branch
                let new_meta = RevisionMeta {
                    branch: target_branch.clone(),
                    ..revision_meta
                };

                // Store the modified revision metadata
                if let Err(e) = self
                    .store_revision_meta(tenant_id, repo_id, &new_meta)
                    .await
                {
                    warn!(
                        job_id = %job.id,
                        revision = %new_meta.revision,
                        error = %e,
                        "Failed to copy revision, continuing with next"
                    );
                    continue;
                }

                total_copied += 1;
            }

            debug!(
                job_id = %job.id,
                batch_offset = offset,
                batch_size = batch_size,
                total_copied = total_copied,
                "Processed revision batch"
            );

            offset += batch_size;

            // If we got fewer than BATCH_SIZE, we've reached the end
            if batch_size < BATCH_SIZE {
                break;
            }
        }

        info!(
            job_id = %job.id,
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            source_branch = %source_branch,
            target_branch = %target_branch,
            total_copied = total_copied,
            "Completed revision history copy"
        );

        Ok(())
    }

    /// List revisions for a specific branch with pagination
    async fn list_revisions_for_branch(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<RevisionMeta>> {
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push("revisions")
            .build_prefix();

        let cf = cf_handle(&self.db, cf::REVISIONS)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        let mut revisions = Vec::new();
        let mut skipped = 0;

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&prefix_clone) {
                break;
            }

            let meta: RevisionMeta = rmp_serde::from_slice(&value).map_err(|e| {
                raisin_error::Error::storage(format!("Deserialization error: {}", e))
            })?;

            // Filter by branch
            if meta.branch != branch {
                continue;
            }

            // Apply offset
            if skipped < offset {
                skipped += 1;
                continue;
            }

            revisions.push(meta);

            // Apply limit
            if revisions.len() >= limit {
                break;
            }
        }

        Ok(revisions)
    }

    /// Store a revision metadata record
    async fn store_revision_meta(
        &self,
        tenant_id: &str,
        repo_id: &str,
        meta: &RevisionMeta,
    ) -> Result<()> {
        let key = keys::revision_meta_key(tenant_id, repo_id, &meta.revision);
        let value = rmp_serde::to_vec(&meta)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        let cf = cf_handle(&self.db, cf::REVISIONS)?;
        self.db
            .put_cf(cf, key, value)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        Ok(())
    }
}
