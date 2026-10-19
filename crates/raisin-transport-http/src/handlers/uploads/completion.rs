// SPDX-License-Identifier: BSL-1.1

//! Upload completion and cancellation handlers.

use axum::{
    extract::{Extension, Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use chrono::Utc;
use std::collections::HashMap;

use raisin_models::auth::AuthContext;

use crate::{error::ApiError, middleware::TenantInfo, state::AppState};

use super::authorize::{authorize_session_write, validate_path};
use super::token::{principal, require_session_access, Principal};

use super::types::{
    CompleteUploadRequest, CompleteUploadResponse, UploadSessionStatus, UPLOAD_STORE,
};

/// POST /api/uploads/{id}/complete - Complete upload
pub async fn complete_upload(
    State(state): State<AppState>,
    Path(upload_id): Path<String>,
    Extension(tenant_info): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    headers: HeaderMap,
    Json(req): Json<CompleteUploadRequest>,
) -> Result<Json<CompleteUploadResponse>, ApiError> {
    // Get session
    let mut session = UPLOAD_STORE
        .get(&upload_id)
        .await
        .ok_or_else(|| ApiError::not_found("Upload session not found"))?;
    let principal = principal(&state, &headers, &tenant_info, auth)?;
    require_session_access(&principal, &tenant_info, &session)?;
    let mut req = req;
    match &principal {
        // Re-checked: the grant, or the node at the path, may have changed
        // since the session opened.
        Principal::User(auth) => authorize_session_write(&state, auth, &session).await?,
        Principal::Token(_) => {
            // Authorized when the token was minted. A token completes once, and
            // does not choose who the commit is attributed to.
            validate_path(&session.path)?;
            if !matches!(
                session.status,
                UploadSessionStatus::Pending | UploadSessionStatus::InProgress
            ) {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "ALREADY_COMPLETED",
                    "Upload already completed",
                ));
            }
            req.commit_actor = Some("upload-token".to_string());
            req.commit_message = None;
        }
    }

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

        // Store job context BEFORE registering so dispatch can never
        // observe the job without its context.
        let job_id = raisin_storage::jobs::JobId::new();
        job_data_store.put(&job_id, &job_context).map_err(|e| {
            ApiError::internal(format!(
                "Failed to store job context for upload completion: {}",
                e
            ))
        })?;

        // Register job under the pre-generated ID
        job_registry
            .register_job_with_id(
                job_id.clone(),
                job_type,
                session.tenant_id.clone(),
                None,
                None,
                None,
            )
            .await
            .map_err(|e| ApiError::internal(format!("Failed to register completion job: {}", e)))?;

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
pub async fn cancel_upload(
    State(state): State<AppState>,
    Path(upload_id): Path<String>,
    Extension(tenant_info): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    // Get session
    let mut session = UPLOAD_STORE
        .get(&upload_id)
        .await
        .ok_or_else(|| ApiError::not_found("Upload session not found"))?;
    let principal = principal(&state, &headers, &tenant_info, auth)?;
    require_session_access(&principal, &tenant_info, &session)?;

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
