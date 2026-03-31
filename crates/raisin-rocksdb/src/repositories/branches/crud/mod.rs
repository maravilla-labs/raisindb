//! Branch CRUD operations
//!
//! Provides basic create, read, update, delete operations for branches,
//! implementing the BranchRepository trait.

mod batch_ops;
mod management;

use crate::{cf, cf_handle, keys};
use raisin_context::{
    Branch, BranchDivergence, ConflictResolution, MergeConflict, MergeResult, MergeStrategy,
};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_storage::jobs::{JobContext, JobType};
use raisin_storage::BranchRepository;
use std::collections::HashMap;

use super::BranchRepositoryImpl;

impl BranchRepository for BranchRepositoryImpl {
    async fn create_branch(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        created_by: &str,
        from_revision: Option<HLC>,
        upstream_branch: Option<String>,
        protected: bool,
        include_revision_history: bool,
    ) -> Result<Branch> {
        // Capture source branch for index copying before moving upstream_branch
        let source_branch_for_indexes = upstream_branch.as_deref().unwrap_or("main").to_string();

        let branch = Branch {
            name: branch_name.to_string(),
            head: from_revision.unwrap_or_else(|| HLC::new(0, 0)),
            created_at: chrono::Utc::now(),
            created_by: created_by.to_string(),
            created_from: from_revision,
            upstream_branch,
            protected,
            description: None,
        };

        let key = keys::branch_key(tenant_id, repo_id, branch_name);
        let value = rmp_serde::to_vec(&branch)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        let cf = cf_handle(&self.db, cf::BRANCHES)?;
        self.db
            .put_cf(cf, key, value)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        // Capture operation for replication WITH the initial branch head as the revision
        if let Some(ref capture) = self.operation_capture {
            if capture.is_enabled() {
                let _op = capture
                    .capture_operation_with_revision(
                        tenant_id.to_string(),
                        repo_id.to_string(),
                        branch_name.to_string(),
                        raisin_replication::OpType::UpdateBranch {
                            branch: branch.clone(),
                        },
                        created_by.to_string(),
                        Some(format!("Branch '{}' created", branch_name)),
                        true,
                        Some(branch.head),
                    )
                    .await;
            }
        }

        // Copy revision-aware indexes from source branch up to from_revision
        if let Some(ref max_revision) = from_revision {
            tracing::info!(
                "Copying indexes from branch '{}' at revision {:?} to new branch '{}'",
                source_branch_for_indexes,
                max_revision,
                branch_name
            );

            self.copy_branch_indexes(
                tenant_id,
                repo_id,
                &source_branch_for_indexes,
                branch_name,
                max_revision,
            )
            .await?;

            // Queue background job to copy revision history if requested
            if include_revision_history {
                if let (Some(job_registry), Some(job_data_store)) =
                    (&self.job_registry, &self.job_data_store)
                {
                    let job_type = JobType::RevisionHistoryCopy {
                        source_branch: source_branch_for_indexes.clone(),
                        target_branch: branch_name.to_string(),
                        up_to_revision: *max_revision,
                    };

                    let context = JobContext {
                        tenant_id: tenant_id.to_string(),
                        repo_id: repo_id.to_string(),
                        branch: branch_name.to_string(),
                        workspace_id: String::new(),
                        revision: *max_revision,
                        metadata: HashMap::new(),
                    };

                    match job_registry
                        .register_job(
                            job_type.clone(),
                            Some(tenant_id.to_string()),
                            None,
                            None,
                            None,
                        )
                        .await
                    {
                        Ok(job_id) => {
                            if let Err(e) = job_data_store.put(&job_id, &context) {
                                tracing::error!(
                                    job_id = %job_id,
                                    error = %e,
                                    "Failed to store job context for revision history copy"
                                );
                            } else {
                                tracing::info!(
                                    job_id = %job_id,
                                    source_branch = %source_branch_for_indexes,
                                    target_branch = %branch_name,
                                    up_to_revision = %max_revision,
                                    "Enqueued revision history copy job"
                                );
                            }
                        }
                        Err(e) => {
                            tracing::error!(
                                error = %e,
                                source_branch = %source_branch_for_indexes,
                                target_branch = %branch_name,
                                "Failed to enqueue revision history copy job"
                            );
                        }
                    }
                } else {
                    tracing::debug!(
                        "Job system not configured, skipping revision history copy job"
                    );
                }
            }
        }

        Ok(branch)
    }

    async fn get_branch(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
    ) -> Result<Option<Branch>> {
        let key = keys::branch_key(tenant_id, repo_id, branch_name);
        let cf = cf_handle(&self.db, cf::BRANCHES)?;

        match self.db.get_cf(cf, &key) {
            Ok(Some(bytes)) => {
                let branch: Branch = rmp_serde::from_slice(&bytes).map_err(|e| {
                    raisin_error::Error::storage(format!("Deserialization error: {}", e))
                })?;
                Ok(Some(branch))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(raisin_error::Error::storage(e.to_string())),
        }
    }

    async fn list_branches(&self, tenant_id: &str, repo_id: &str) -> Result<Vec<Branch>> {
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push("branches")
            .build_prefix();

        let cf = cf_handle(&self.db, cf::BRANCHES)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        let mut branches = Vec::new();

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            if !key.starts_with(&prefix_clone) {
                break;
            }

            let branch: Branch = rmp_serde::from_slice(&value).map_err(|e| {
                raisin_error::Error::storage(format!("Deserialization error: {}", e))
            })?;
            branches.push(branch);
        }

        Ok(branches)
    }

    async fn delete_branch(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
    ) -> Result<bool> {
        let key = keys::branch_key(tenant_id, repo_id, branch_name);
        let cf = cf_handle(&self.db, cf::BRANCHES)?;

        let branch_opt = self
            .db
            .get_cf(cf, &key)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?
            .map(|bytes| {
                rmp_serde::from_slice::<Branch>(&bytes).map_err(|e| {
                    raisin_error::Error::storage(format!("Deserialization error: {}", e))
                })
            })
            .transpose()?;

        if let Some(branch) = branch_opt {
            if branch.protected {
                return Err(raisin_error::Error::Forbidden(format!(
                    "Cannot delete protected branch '{}'",
                    branch_name
                )));
            }

            self.db
                .delete_cf(cf, key)
                .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            if let Some(ref capture) = self.operation_capture {
                if capture.is_enabled() {
                    let _op = capture
                        .capture_operation_with_revision(
                            tenant_id.to_string(),
                            repo_id.to_string(),
                            branch_name.to_string(),
                            raisin_replication::OpType::DeleteBranch {
                                branch_id: branch_name.to_string(),
                            },
                            "system".to_string(),
                            Some(format!("Branch '{}' deleted", branch_name)),
                            true,
                            Some(branch.head),
                        )
                        .await;
                }
            }

            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn get_head(&self, tenant_id: &str, repo_id: &str, branch_name: &str) -> Result<HLC> {
        let branch = self
            .get_branch(tenant_id, repo_id, branch_name)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Branch '{}' not found", branch_name))
            })?;

        Ok(branch.head)
    }

    async fn update_head(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        new_head: HLC,
    ) -> Result<()> {
        let mut branch = self
            .get_branch(tenant_id, repo_id, branch_name)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Branch '{}' not found", branch_name))
            })?;

        if branch.protected {
            return Err(raisin_error::Error::Forbidden(format!(
                "Cannot modify protected branch '{}': branch is protected from commits",
                branch_name
            )));
        }

        eprintln!(
            "update_head: branch={}, old_head={:?}, new_head={:?}",
            branch_name, branch.head, new_head
        );
        branch.head = new_head;

        let key = keys::branch_key(tenant_id, repo_id, branch_name);
        let value = rmp_serde::to_vec(&branch)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        let cf = cf_handle(&self.db, cf::BRANCHES)?;
        self.db
            .put_cf(cf, key, value)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        if let Some(ref capture) = self.operation_capture {
            if capture.is_enabled() {
                let _op = capture
                    .capture_operation_with_revision(
                        tenant_id.to_string(),
                        repo_id.to_string(),
                        branch_name.to_string(),
                        raisin_replication::OpType::UpdateBranch {
                            branch: branch.clone(),
                        },
                        "system".to_string(),
                        Some(format!("Branch '{}' head updated", branch_name)),
                        true,
                        Some(new_head),
                    )
                    .await;
            }
        }

        Ok(())
    }

    async fn set_upstream_branch(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        upstream: Option<String>,
    ) -> Result<()> {
        BranchRepositoryImpl::set_upstream_branch(
            self,
            tenant_id,
            repo_id,
            branch_name,
            upstream.as_deref(),
        )
        .await?;
        Ok(())
    }

    async fn set_protected(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        protected: bool,
    ) -> Result<()> {
        BranchRepositoryImpl::set_protected(self, tenant_id, repo_id, branch_name, protected)
            .await?;
        Ok(())
    }

    async fn set_description(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        description: Option<String>,
    ) -> Result<()> {
        BranchRepositoryImpl::set_description(
            self,
            tenant_id,
            repo_id,
            branch_name,
            description.as_deref(),
        )
        .await?;
        Ok(())
    }

    async fn calculate_divergence(
        &self,
        tenant_id: &str,
        repo_id: &str,
        current_branch: &str,
        base_branch: &str,
    ) -> Result<BranchDivergence> {
        BranchRepositoryImpl::calculate_divergence(
            self,
            tenant_id,
            repo_id,
            current_branch,
            base_branch,
        )
        .await
    }

    async fn merge_branches(
        &self,
        tenant_id: &str,
        repo_id: &str,
        target_branch: &str,
        source_branch: &str,
        strategy: MergeStrategy,
        message: &str,
        actor: &str,
    ) -> Result<MergeResult> {
        BranchRepositoryImpl::merge_branches(
            self,
            tenant_id,
            repo_id,
            target_branch,
            source_branch,
            strategy,
            message,
            actor,
        )
        .await
    }

    async fn find_merge_conflicts(
        &self,
        tenant_id: &str,
        repo_id: &str,
        target_branch: &str,
        source_branch: &str,
    ) -> Result<Vec<MergeConflict>> {
        BranchRepositoryImpl::find_merge_conflicts(
            self,
            tenant_id,
            repo_id,
            target_branch,
            source_branch,
        )
        .await
    }

    async fn resolve_merge_with_resolutions(
        &self,
        tenant_id: &str,
        repo_id: &str,
        target_branch: &str,
        source_branch: &str,
        resolutions: Vec<ConflictResolution>,
        message: &str,
        actor: &str,
    ) -> Result<MergeResult> {
        BranchRepositoryImpl::resolve_merge_with_resolutions(
            self,
            tenant_id,
            repo_id,
            target_branch,
            source_branch,
            resolutions,
            message,
            actor,
        )
        .await
    }
}
