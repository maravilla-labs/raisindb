//! Core fulltext job handler for single-node and branch-copy operations.

use raisin_error::{Error, Result};
use raisin_indexer::TantivyIndexingEngine;
use raisin_models::auth::AuthContext;
use raisin_storage::fulltext::IndexingEngine;
use raisin_storage::jobs::{IndexOperation, JobContext, JobInfo, JobType};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{FullTextIndexJob, JobKind, RepositoryManagementRepository, Storage};
use std::sync::Arc;

use crate::jobs::{IndexKey, IndexLockManager};
use crate::RocksDBStorage;

/// Handler for fulltext indexing jobs
///
/// This handler processes fulltext index operations by:
/// 1. Extracting job parameters from JobType enum variants
/// 2. Fetching node data from storage at exact revisions
/// 3. Getting repository language configuration
/// 4. Delegating to TantivyIndexingEngine for actual indexing
///
/// Includes index-level locking to prevent concurrent writes to the same
/// Tantivy index directory, avoiding LockBusy errors.
pub struct FulltextJobHandler {
    pub(super) storage: Arc<RocksDBStorage>,
    pub(super) tantivy_engine: Arc<TantivyIndexingEngine>,
    pub(super) index_lock_manager: Arc<IndexLockManager>,
}

impl FulltextJobHandler {
    /// Create a new fulltext job handler
    pub fn new(
        storage: Arc<RocksDBStorage>,
        tantivy_engine: Arc<TantivyIndexingEngine>,
        index_lock_manager: Arc<IndexLockManager>,
    ) -> Self {
        Self {
            storage,
            tantivy_engine,
            index_lock_manager,
        }
    }

    /// Handle fulltext index job (add/update/delete node)
    pub async fn handle_index(&self, job: &JobInfo, context: &JobContext) -> Result<()> {
        let (node_id, operation) = match &job.job_type {
            JobType::FulltextIndex { node_id, operation } => (node_id, operation),
            _ => {
                return Err(Error::Validation(
                    "Expected FulltextIndex job type".to_string(),
                ))
            }
        };

        tracing::debug!(
            job_id = %job.id,
            node_id = %node_id,
            operation = ?operation,
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            branch = %context.branch,
            revision = %context.revision,
            "Processing fulltext index job"
        );

        // Acquire lock for this index to prevent concurrent writes
        let index_key = IndexKey::new(&context.tenant_id, &context.repo_id, &context.branch);
        let index_lock = self.index_lock_manager.get_lock(&index_key).await;
        let _lock_guard = index_lock.lock().await;

        match operation {
            IndexOperation::AddOrUpdate => self.handle_add_or_update(job, context, node_id).await,
            IndexOperation::Delete => self.handle_delete(job, context, node_id).await,
        }
    }

    /// Handle branch copy job
    pub async fn handle_branch_copy(&self, job: &JobInfo, context: &JobContext) -> Result<()> {
        let source_branch = match &job.job_type {
            JobType::FulltextBranchCopy { source_branch } => source_branch,
            _ => {
                return Err(Error::Validation(
                    "Expected FulltextBranchCopy job type".to_string(),
                ))
            }
        };

        tracing::debug!(
            job_id = %job.id,
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            target_branch = %context.branch,
            source_branch = %source_branch,
            "Processing fulltext branch copy job"
        );

        // Acquire lock for the target branch index
        let index_key = IndexKey::new(&context.tenant_id, &context.repo_id, &context.branch);
        let index_lock = self.index_lock_manager.get_lock(&index_key).await;
        let _lock_guard = index_lock.lock().await;

        // Get repository configuration for language settings
        let repo_info = self
            .storage
            .repository_management()
            .get_repository(&context.tenant_id, &context.repo_id)
            .await?
            .ok_or_else(|| {
                Error::NotFound(format!(
                    "Repository not found: {}/{}",
                    context.tenant_id, context.repo_id
                ))
            })?;

        let legacy_job = FullTextIndexJob {
            job_id: job.id.to_string(),
            kind: JobKind::BranchCreated,
            tenant_id: context.tenant_id.clone(),
            repo_id: context.repo_id.clone(),
            workspace_id: context.workspace_id.clone(),
            branch: context.branch.clone(),
            revision: context.revision,
            node_id: None,
            source_branch: Some(source_branch.clone()),
            default_language: repo_info.config.default_language,
            supported_languages: repo_info.config.supported_languages,
            properties_to_index: None,
        };

        let engine = Arc::clone(&self.tantivy_engine);
        let job_clone = legacy_job.clone();
        let branch = context.branch.clone();
        let source_branch_clone = source_branch.clone();

        tokio::task::spawn_blocking(move || engine.do_branch_created(&job_clone))
            .await
            .map_err(|e| Error::storage(format!("Blocking task failed: {}", e)))??;

        tracing::debug!(
            job_id = %job.id,
            target_branch = %branch,
            source_branch = %source_branch_clone,
            "Fulltext index copied successfully"
        );

        Ok(())
    }

    /// Handle add or update operation
    pub(super) async fn handle_add_or_update(
        &self,
        job: &JobInfo,
        context: &JobContext,
        node_id: &str,
    ) -> Result<()> {
        // Create transaction with system auth context for node reads
        let tx = self.storage.begin_context().await?;
        tx.set_tenant_repo(&context.tenant_id, &context.repo_id)?;
        tx.set_branch(&context.branch)?;
        tx.set_actor("fulltext-indexer")?;
        tx.set_auth_context(AuthContext::system())?;

        // Fetch node from storage at exact revision
        let node = tx
            .get_node(&context.workspace_id, node_id)
            .await?
            .ok_or_else(|| {
                Error::NotFound(format!(
                    "Node {} not found at revision {}",
                    node_id, context.revision
                ))
            })?;

        // Get repository configuration for language settings
        let repo_info = self
            .storage
            .repository_management()
            .get_repository(&context.tenant_id, &context.repo_id)
            .await?
            .ok_or_else(|| {
                Error::NotFound(format!(
                    "Repository not found: {}/{}",
                    context.tenant_id, context.repo_id
                ))
            })?;

        let legacy_job = FullTextIndexJob {
            job_id: job.id.to_string(),
            kind: JobKind::AddNode,
            tenant_id: context.tenant_id.clone(),
            repo_id: context.repo_id.clone(),
            workspace_id: context.workspace_id.clone(),
            branch: context.branch.clone(),
            revision: context.revision,
            node_id: Some(node_id.to_string()),
            source_branch: None,
            default_language: repo_info.config.default_language,
            supported_languages: repo_info.config.supported_languages,
            properties_to_index: None,
        };

        let engine = Arc::clone(&self.tantivy_engine);
        let job_clone = legacy_job.clone();
        let node_clone = node.clone();

        tokio::task::spawn_blocking(move || engine.do_index_node(&job_clone, &node_clone))
            .await
            .map_err(|e| Error::storage(format!("Blocking task failed: {}", e)))??;

        tracing::trace!(
            job_id = %job.id,
            node_id = %node_id,
            "Node indexed successfully"
        );

        Ok(())
    }

    /// Handle delete operation
    pub(super) async fn handle_delete(
        &self,
        job: &JobInfo,
        context: &JobContext,
        node_id: &str,
    ) -> Result<()> {
        // Get repository configuration for language settings
        let repo_info = self
            .storage
            .repository_management()
            .get_repository(&context.tenant_id, &context.repo_id)
            .await?
            .ok_or_else(|| {
                Error::NotFound(format!(
                    "Repository not found: {}/{}",
                    context.tenant_id, context.repo_id
                ))
            })?;

        let legacy_job = FullTextIndexJob {
            job_id: job.id.to_string(),
            kind: JobKind::DeleteNode,
            tenant_id: context.tenant_id.clone(),
            repo_id: context.repo_id.clone(),
            workspace_id: context.workspace_id.clone(),
            branch: context.branch.clone(),
            revision: context.revision,
            node_id: Some(node_id.to_string()),
            source_branch: None,
            default_language: repo_info.config.default_language,
            supported_languages: repo_info.config.supported_languages,
            properties_to_index: None,
        };

        let engine = Arc::clone(&self.tantivy_engine);
        let job_clone = legacy_job.clone();

        tokio::task::spawn_blocking(move || engine.do_delete_node(&job_clone))
            .await
            .map_err(|e| Error::storage(format!("Blocking task failed: {}", e)))??;

        tracing::trace!(
            job_id = %job.id,
            node_id = %node_id,
            "Node deleted from fulltext index successfully"
        );

        Ok(())
    }
}
