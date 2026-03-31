// SPDX-License-Identifier: BSL-1.1

//! Upload session creation handler.

use axum::{
    extract::{Extension, State},
    http::StatusCode,
    Json,
};
use chrono::{Duration, Utc};

use crate::{error::ApiError, middleware::TenantInfo, state::AppState};

use super::types::{
    CreateUploadRequest, CreateUploadResponse, UploadSession, UploadSessionStatus,
    DEFAULT_SESSION_EXPIRATION_HOURS, UPLOAD_STORE, UPLOAD_TEMP_DIR,
};

/// POST /api/uploads - Create upload session
pub async fn create_upload(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    Json(req): Json<CreateUploadRequest>,
) -> Result<(StatusCode, Json<CreateUploadResponse>), ApiError> {
    let tenant_id = &tenant_info.tenant_id;

    // Validate inputs
    if req.file_size == 0 {
        return Err(ApiError::validation_failed(
            "File size must be greater than 0",
        ));
    }

    if req.chunk_size == 0 || req.chunk_size > 100 * 1024 * 1024 {
        return Err(ApiError::validation_failed(
            "Chunk size must be between 1 byte and 100MB",
        ));
    }

    // Calculate total chunks
    let total_chunks = req.file_size.div_ceil(req.chunk_size) as u32;

    // Generate upload ID
    let upload_id = nanoid::nanoid!();

    // Create temp directory path
    let temp_dir = format!("{}/{}", UPLOAD_TEMP_DIR, upload_id);

    // Create session
    let now = Utc::now();
    let expires_at = now + Duration::hours(DEFAULT_SESSION_EXPIRATION_HOURS);

    let session = UploadSession {
        id: upload_id.clone(),
        tenant_id: tenant_id.to_string(),
        repository: req.repository.clone(),
        branch: req.branch.clone(),
        workspace: req.workspace.clone(),
        path: req.path.clone(),
        filename: req.filename.clone(),
        file_size: req.file_size,
        content_type: req.content_type.clone(),
        node_type: req.node_type.clone(),
        chunk_size: req.chunk_size,
        bytes_received: 0,
        chunks_completed: 0,
        total_chunks,
        status: UploadSessionStatus::Pending,
        temp_dir: temp_dir.clone(),
        metadata: req.metadata.clone(),
        created_at: now,
        updated_at: now,
        expires_at,
    };

    // Store session
    UPLOAD_STORE.put(session.clone()).await;

    tracing::info!(
        upload_id = %upload_id,
        file_size = req.file_size,
        total_chunks = total_chunks,
        "Created upload session"
    );

    Ok((
        StatusCode::CREATED,
        Json(CreateUploadResponse {
            upload_id: upload_id.clone(),
            upload_url: format!("/api/uploads/{}", upload_id),
            chunk_size: req.chunk_size,
            total_chunks,
            expires_at,
        }),
    ))
}
