//! Batch fulltext index operations.
//!
//! Contains `handle_batch_index` for processing multiple node operations
//! in a single Tantivy commit for dramatically improved bulk import performance.

use super::handler::FulltextJobHandler;
use raisin_error::{Error, Result};
use raisin_indexer::BatchIndexContext;
use raisin_models::auth::AuthContext;
use raisin_storage::jobs::{BatchIndexOperation, IndexOperation, JobContext, JobInfo, JobType};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{RepositoryManagementRepository, Storage};
use std::sync::Arc;

use crate::jobs::IndexKey;

impl FulltextJobHandler {
    /// Handle batch fulltext index job (multiple nodes, single commit)
    ///
    /// Processes a FulltextBatchIndex job which contains multiple node operations
    /// and processes them all with a single Tantivy commit for dramatically improved
    /// bulk import performance.
    ///
    /// # Performance
    ///
    /// For 1000 nodes:
    /// - Single-node jobs: 1000 commits x ~50ms = ~50 seconds
    /// - Batch job: 1 commit x ~50ms = ~1 second (50x faster)
    pub async fn handle_batch_index(&self, job: &JobInfo, context: &JobContext) -> Result<()> {
        let operation_count = match &job.job_type {
            JobType::FulltextBatchIndex { operation_count } => *operation_count,
            _ => {
                return Err(Error::Validation(
                    "Expected FulltextBatchIndex job type".to_string(),
                ))
            }
        };

        // Extract batch operations from context metadata
        let operations: Vec<BatchIndexOperation> = context
            .metadata
            .get("batch_operations")
            .ok_or_else(|| {
                Error::Validation("Missing batch_operations in job context metadata".to_string())
            })
            .and_then(|v| {
                serde_json::from_value(v.clone()).map_err(|e| {
                    Error::Validation(format!("Failed to deserialize batch_operations: {}", e))
                })
            })?;

        if operations.is_empty() {
            tracing::warn!(job_id = %job.id, "Batch index job has no operations");
            return Ok(());
        }

        tracing::info!(
            job_id = %job.id,
            operation_count = operation_count,
            actual_operations = operations.len(),
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            branch = %context.branch,
            "Processing fulltext batch index job"
        );

        // Acquire lock ONCE for entire batch
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

        // Create transaction with system auth context for node reads
        let tx = self.storage.begin_context().await?;
        tx.set_tenant_repo(&context.tenant_id, &context.repo_id)?;
        tx.set_branch(&context.branch)?;
        tx.set_actor("fulltext-indexer")?;
        tx.set_auth_context(AuthContext::system())?;

        // Collect nodes for add/update operations
        let mut nodes_to_index = Vec::new();
        let mut delete_node_ids = Vec::new();

        for op in &operations {
            match &op.operation {
                IndexOperation::AddOrUpdate => {
                    match tx.get_node(&context.workspace_id, &op.node_id).await {
                        Ok(Some(node)) => nodes_to_index.push(node),
                        Ok(None) => {
                            tracing::warn!(
                                job_id = %job.id,
                                node_id = %op.node_id,
                                "Node not found for batch indexing, skipping"
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                job_id = %job.id,
                                node_id = %op.node_id,
                                error = %e,
                                "Failed to fetch node for batch indexing, skipping"
                            );
                        }
                    }
                }
                IndexOperation::Delete => {
                    delete_node_ids.push(op.node_id.clone());
                }
            }
        }

        let index_count = nodes_to_index.len();
        let delete_count = delete_node_ids.len();

        // Create batch context
        let batch_context = BatchIndexContext {
            tenant_id: context.tenant_id.clone(),
            repo_id: context.repo_id.clone(),
            branch: context.branch.clone(),
            workspace_id: context.workspace_id.clone(),
            default_language: repo_info.config.default_language.clone(),
            supported_languages: repo_info.config.supported_languages.clone(),
        };

        // Execute batch indexing in blocking task (Tantivy is sync)
        let engine = Arc::clone(&self.tantivy_engine);

        let result = tokio::task::spawn_blocking(move || {
            engine.do_batch_index(&batch_context, nodes_to_index, delete_node_ids)
        })
        .await
        .map_err(|e| Error::storage(format!("Blocking task failed: {}", e)))??;

        tracing::info!(
            job_id = %job.id,
            indexed = index_count,
            deleted = delete_count,
            processed = result,
            "Batch index completed successfully"
        );

        Ok(())
    }
}
