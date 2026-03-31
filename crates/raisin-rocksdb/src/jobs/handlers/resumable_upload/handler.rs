//! Main resumable upload handler.
//!
//! Contains the `ResumableUploadHandler` struct, its constructor, builder,
//! the main `handle` method, and helper methods for chunk validation,
//! upload, cleanup, and progress reporting.

use anyhow::Context;
use raisin_error::{Error, Result};
use raisin_storage::jobs::{JobContext, JobId, JobInfo, JobRegistry, JobType};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::upload_sessions::{UploadSession, UploadSessionStatus};
use raisin_storage::Storage;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;

/// Callback type for binary storage (uploading assembled chunks)
///
/// This callback is provided by the transport layer which has access to BinaryStorage.
/// Arguments: (chunk_paths, filename, content_type, tenant_id, file_size)
/// Returns: Result<StoredObject> - metadata about stored binary
pub type BinaryUploadCallback = Arc<
    dyn Fn(
            Vec<PathBuf>,   // chunk_paths to read sequentially
            String,         // filename
            Option<String>, // content_type
            String,         // tenant_id
            u64,            // file_size
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<raisin_binary::StoredObject>> + Send>,
        > + Send
        + Sync,
>;

/// Handler for resumable upload completion jobs
///
/// This handler orchestrates the final assembly of chunked uploads by:
/// - Reading all uploaded chunks from temporary storage
/// - Streaming them to BinaryStorage via callback
/// - Creating or updating the target node with the uploaded file
/// - Cleaning up temporary files
pub struct ResumableUploadHandler<S: Storage> {
    pub(super) storage: Arc<S>,
    pub(super) job_registry: Arc<JobRegistry>,
    pub(super) binary_upload_callback: Option<BinaryUploadCallback>,
}

impl<S: Storage + TransactionalStorage> ResumableUploadHandler<S> {
    /// Create a new resumable upload handler
    pub fn new(storage: Arc<S>, job_registry: Arc<JobRegistry>) -> Self {
        Self {
            storage,
            job_registry,
            binary_upload_callback: None,
        }
    }

    /// Set the binary upload callback
    pub fn with_binary_upload_callback(mut self, callback: BinaryUploadCallback) -> Self {
        self.binary_upload_callback = Some(callback);
        self
    }

    /// Set the binary upload callback (mutable reference)
    pub fn set_binary_upload_callback(&mut self, callback: BinaryUploadCallback) {
        self.binary_upload_callback = Some(callback);
    }

    /// Handle resumable upload completion job
    pub async fn handle(
        &self,
        job: &JobInfo,
        context: &JobContext,
    ) -> Result<Option<serde_json::Value>> {
        // Extract parameters from JobType
        let (upload_id, commit_message, commit_actor) = match &job.job_type {
            JobType::ResumableUploadComplete {
                upload_id,
                commit_message,
                commit_actor,
            } => (upload_id, commit_message, commit_actor),
            _ => {
                return Err(Error::Validation(
                    "Expected ResumableUploadComplete job type".to_string(),
                ))
            }
        };

        tracing::info!(
            job_id = %job.id,
            upload_id = %upload_id,
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            "Starting resumable upload completion"
        );

        // Get upload session metadata from job context
        let session = self.get_session_from_context(context)?;

        // Report progress: validating chunks
        self.report_progress(&job.id, 0.1, "Validating chunks")
            .await;

        // Validate all chunks exist
        self.validate_chunks(&session, &job.id).await?;

        // Report progress: assembling file
        self.report_progress(&job.id, 0.3, "Assembling file").await;

        // Get binary upload callback
        let binary_upload = self.binary_upload_callback.as_ref().ok_or_else(|| {
            Error::Validation("Binary upload callback not configured".to_string())
        })?;

        // Upload assembled chunks to binary storage
        let stored = self
            .upload_chunks_to_storage(&session, binary_upload, &job.id)
            .await?;

        tracing::info!(
            job_id = %job.id,
            upload_id = %upload_id,
            storage_key = %stored.key,
            "Chunks assembled and uploaded to storage"
        );

        // Report progress: creating node
        self.report_progress(&job.id, 0.7, "Creating node").await;

        // Create or update node with Resource property
        let node_id = self
            .create_or_update_node(&session, &stored, context, commit_message, commit_actor)
            .await?;

        tracing::info!(
            job_id = %job.id,
            upload_id = %upload_id,
            node_id = %node_id,
            path = %session.path,
            "Node created/updated successfully"
        );

        // Report progress: cleaning up
        self.report_progress(&job.id, 0.9, "Cleaning up temporary files")
            .await;

        // Delete temp directory with all chunks
        self.cleanup_temp_files(&session, &job.id).await;

        // Update session status (note: in-memory store for now)
        // TODO: When upload sessions are persisted in RocksDB, update status there
        tracing::info!(
            job_id = %job.id,
            upload_id = %upload_id,
            "Upload session marked as completed"
        );

        self.report_progress(&job.id, 1.0, "Upload complete").await;

        let result = serde_json::json!({
            "upload_id": upload_id,
            "node_id": node_id,
            "path": session.path,
            "file_size": session.file_size,
            "status": "completed"
        });

        Ok(Some(result))
    }

    /// Get upload session from job context metadata
    pub(super) fn get_session_from_context(&self, context: &JobContext) -> Result<UploadSession> {
        // Extract session fields from metadata
        let upload_id = context
            .metadata
            .get("upload_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Validation("Missing upload_id in context".to_string()))?
            .to_string();

        let filename = context
            .metadata
            .get("filename")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Validation("Missing filename in context".to_string()))?
            .to_string();

        let path = context
            .metadata
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Validation("Missing path in context".to_string()))?
            .to_string();

        let temp_dir = context
            .metadata
            .get("temp_dir")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Validation("Missing temp_dir in context".to_string()))?
            .to_string();

        let file_size = context
            .metadata
            .get("file_size")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| Error::Validation("Missing file_size in context".to_string()))?;

        let total_chunks = context
            .metadata
            .get("total_chunks")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| Error::Validation("Missing total_chunks in context".to_string()))?
            as u32;

        let content_type = context
            .metadata
            .get("content_type")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let node_type = context
            .metadata
            .get("node_type")
            .and_then(|v| v.as_str())
            .unwrap_or("raisin:Asset")
            .to_string();

        // Reconstruct session for processing
        Ok(UploadSession {
            id: upload_id.clone(),
            tenant_id: context.tenant_id.clone(),
            repository: context.repo_id.clone(),
            branch: context.branch.clone(),
            workspace: context.workspace_id.clone(),
            path,
            filename,
            file_size,
            content_type,
            node_type,
            chunk_size: 10 * 1024 * 1024, // Default chunk size
            bytes_received: file_size,    // All bytes should be received
            chunks_completed: total_chunks,
            total_chunks,
            status: UploadSessionStatus::Completing,
            temp_dir,
            metadata: context
                .metadata
                .get("user_metadata")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: chrono::Utc::now() + chrono::Duration::hours(24),
            error: None,
            created_by: None,
        })
    }

    /// Validate that all chunk files exist
    async fn validate_chunks(&self, session: &UploadSession, job_id: &JobId) -> Result<()> {
        tracing::debug!(
            job_id = %job_id,
            total_chunks = session.total_chunks,
            temp_dir = %session.temp_dir,
            "Validating chunk files"
        );

        for chunk_num in 0..session.total_chunks {
            let chunk_path = PathBuf::from(session.chunk_filename(chunk_num));

            if !chunk_path.exists() {
                let error_msg = format!("Missing chunk file: {:?}", chunk_path);
                tracing::error!(
                    job_id = %job_id,
                    chunk_num = chunk_num,
                    chunk_path = ?chunk_path,
                    "Chunk file not found"
                );
                return Err(Error::NotFound(error_msg));
            }

            tracing::trace!(
                job_id = %job_id,
                chunk_num = chunk_num,
                "Chunk file validated"
            );
        }

        tracing::debug!(
            job_id = %job_id,
            total_chunks = session.total_chunks,
            "All chunk files validated successfully"
        );

        Ok(())
    }

    /// Upload chunks to binary storage via callback
    async fn upload_chunks_to_storage(
        &self,
        session: &UploadSession,
        binary_upload: &BinaryUploadCallback,
        job_id: &JobId,
    ) -> Result<raisin_binary::StoredObject> {
        tracing::debug!(
            job_id = %job_id,
            total_chunks = session.total_chunks,
            file_size = session.file_size,
            "Preparing chunks for upload"
        );

        // Collect chunk paths
        let chunk_paths: Vec<PathBuf> = (0..session.total_chunks)
            .map(|i| PathBuf::from(session.chunk_filename(i)))
            .collect();

        tracing::debug!(
            job_id = %job_id,
            filename = %session.filename,
            content_type = ?session.content_type,
            "Uploading to binary storage via callback"
        );

        // Call the binary upload callback with chunk paths
        let stored = binary_upload(
            chunk_paths,
            session.filename.clone(),
            session.content_type.clone(),
            session.tenant_id.clone(),
            session.file_size,
        )
        .await
        .context("Failed to upload chunks to binary storage")?;

        tracing::info!(
            job_id = %job_id,
            storage_key = %stored.key,
            size = stored.size,
            "Chunks uploaded successfully"
        );

        Ok(stored)
    }

    /// Clean up temporary chunk files
    pub(super) async fn cleanup_temp_files(&self, session: &UploadSession, job_id: &JobId) {
        let temp_dir = PathBuf::from(&session.temp_dir);

        if !temp_dir.exists() {
            tracing::debug!(
                job_id = %job_id,
                temp_dir = ?temp_dir,
                "Temp directory does not exist, skipping cleanup"
            );
            return;
        }

        match fs::remove_dir_all(&temp_dir).await {
            Ok(_) => {
                tracing::info!(
                    job_id = %job_id,
                    temp_dir = ?temp_dir,
                    "Temp directory cleaned up successfully"
                );
            }
            Err(e) => {
                tracing::warn!(
                    job_id = %job_id,
                    temp_dir = ?temp_dir,
                    error = %e,
                    "Failed to delete temp directory (non-fatal)"
                );
            }
        }
    }

    /// Report progress to job registry
    pub(super) async fn report_progress(&self, job_id: &JobId, progress: f32, message: &str) {
        tracing::debug!(
            job_id = %job_id,
            progress = %progress,
            message = %message,
            "Upload completion progress"
        );

        if let Err(e) = self.job_registry.update_progress(job_id, progress).await {
            tracing::warn!(
                job_id = %job_id,
                error = %e,
                "Failed to update job progress"
            );
        }
    }
}
