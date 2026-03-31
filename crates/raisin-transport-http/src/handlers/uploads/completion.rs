// SPDX-License-Identifier: BSL-1.1

//! Upload completion and cancellation handlers.

use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    Json,
};
use chrono::Utc;
use std::collections::HashMap;

use crate::{error::ApiError, middleware::TenantInfo, state::AppState};

use super::types::{
    CompleteUploadRequest, CompleteUploadResponse, UploadSessionStatus, UPLOAD_STORE,
};

/// POST /api/uploads/{id}/complete - Complete upload
pub async fn complete_upload(
    State(state): State<AppState>,
    Path(upload_id): Path<String>,
    Extension(tenant_info): Extension<TenantInfo>,
    Json(req): Json<CompleteUploadRequest>,
) -> Result<Json<CompleteUploadResponse>, ApiError> {
    // Get session
    let mut session = UPLOAD_STORE
        .get(&upload_id)
        .await
        .ok_or_else(|| ApiError::not_found("Upload session not found"))?;

    // Check if all chunks have been received
    if session.bytes_received != session.file_size {
        return Err(ApiError::validation_failed(format!(
            "Upload incomplete: {} of {} bytes received",
            session.bytes_received, session.file_size
        )));
    }

    if session.chunks_completed != session.total_chunks {
        return Err(ApiError::validation_failed(format!(
            "Upload incomplete: {} of {} chunks received",
            session.chunks_completed, session.total_chunks
        )));
    }

    // Mark session as completing
    session.status = UploadSessionStatus::Completing;
    session.updated_at = Utc::now();
    UPLOAD_STORE.put(session.clone()).await;

    // Get job registry from RocksDB storage (RocksDB feature only)
    #[cfg(feature = "storage-rocksdb")]
    let (job_registry, job_data_store) = {
        let rocksdb_storage = state
            .rocksdb_storage
            .as_ref()
            .ok_or_else(|| ApiError::internal("RocksDB storage not available"))?;

        let job_registry = rocksdb_storage.job_registry();
        let job_data_store = rocksdb_storage.job_data_store();

        (job_registry, job_data_store)
    };

    #[cfg(not(feature = "storage-rocksdb"))]
    {
        return Err(ApiError::internal(
            "Resumable uploads require RocksDB storage feature",
        ));
    }

    #[cfg(feature = "storage-rocksdb")]
    {
        use raisin_storage::jobs::{JobContext, JobType};

        // Create job for upload completion
        let job_type = JobType::ResumableUploadComplete {
            upload_id: upload_id.clone(),
            commit_message: req.commit_message.clone(),
            commit_actor: req.commit_actor.clone(),
        };

        // Register job
        let job_id = job_registry
            .register_job(job_type, Some(session.tenant_id.clone()), None, None, None)
            .await
            .map_err(|e| ApiError::internal(format!("Failed to register completion job: {}", e)))?;

        // Create job context
        let job_context = JobContext {
            tenant_id: session.tenant_id.clone(),
            repo_id: session.repository.clone(),
            branch: session.branch.clone(),
            workspace_id: session.workspace.clone(),
            revision: raisin_hlc::HLC::now(),
            metadata: {
                let mut metadata = HashMap::new();
                metadata.insert("upload_id".to_string(), serde_json::json!(upload_id));
                metadata.insert("path".to_string(), serde_json::json!(session.path));
                metadata.insert("filename".to_string(), serde_json::json!(session.filename));
                metadata.insert(
                    "content_type".to_string(),
                    serde_json::json!(session.content_type),
                );
                metadata.insert(
                    "node_type".to_string(),
                    serde_json::json!(session.node_type),
                );
                metadata.insert("temp_dir".to_string(), serde_json::json!(session.temp_dir));
                metadata.insert(
                    "file_size".to_string(),
                    serde_json::json!(session.file_size),
                );
                metadata.insert(
                    "total_chunks".to_string(),
                    serde_json::json!(session.total_chunks),
                );
                if !session.metadata.is_null() {
                    metadata.insert("user_metadata".to_string(), session.metadata.clone());
                }
                metadata
            },
        };

        // Store job context
        if let Err(e) = job_data_store.put(&job_id, &job_context) {
            tracing::warn!(
                job_id = %job_id,
                upload_id = %upload_id,
                error = %e,
                "Failed to store job context for upload completion"
            );
        }

        tracing::info!(
            upload_id = %upload_id,
            job_id = %job_id,
            "Enqueued ResumableUploadComplete job"
        );

        Ok(Json(CompleteUploadResponse {
            upload_id: upload_id.clone(),
            job_id: job_id.to_string(),
            status: "completing".to_string(),
        }))
    }
}

/// DELETE /api/uploads/{id} - Cancel upload
pub async fn cancel_upload(Path(upload_id): Path<String>) -> Result<StatusCode, ApiError> {
    // Get session
    let mut session = UPLOAD_STORE
        .get(&upload_id)
        .await
        .ok_or_else(|| ApiError::not_found("Upload session not found"))?;

    // Mark as cancelled
    session.status = UploadSessionStatus::Cancelled;
    session.updated_at = Utc::now();
    UPLOAD_STORE.put(session.clone()).await;

    // Delete temp files
    if tokio::fs::metadata(&session.temp_dir).await.is_ok() {
        if let Err(e) = tokio::fs::remove_dir_all(&session.temp_dir).await {
            tracing::warn!(
                upload_id = %upload_id,
                temp_dir = %session.temp_dir,
                error = %e,
                "Failed to delete temp directory"
            );
        }
    }

    tracing::info!(upload_id = %upload_id, "Upload cancelled");

    Ok(StatusCode::NO_CONTENT)
}
