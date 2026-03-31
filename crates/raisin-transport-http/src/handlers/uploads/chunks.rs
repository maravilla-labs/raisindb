// SPDX-License-Identifier: BSL-1.1

//! Chunk upload handler and Content-Range parsing.

use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use chrono::Utc;
use futures_util::StreamExt;

use crate::{error::ApiError, state::AppState};

use super::types::{ChunkUploadResponse, UploadSessionStatus, UPLOAD_STORE};

/// PATCH /api/uploads/{id} - Upload chunk
pub async fn upload_chunk(
    State(state): State<AppState>,
    Path(upload_id): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Result<Json<ChunkUploadResponse>, ApiError> {
    // Get session
    let mut session = UPLOAD_STORE
        .get(&upload_id)
        .await
        .ok_or_else(|| ApiError::not_found("Upload session not found"))?;

    // Check if session is expired
    if Utc::now() > session.expires_at {
        session.status = UploadSessionStatus::Expired;
        UPLOAD_STORE.put(session.clone()).await;
        return Err(ApiError::new(
            StatusCode::GONE,
            "SESSION_EXPIRED",
            "Upload session has expired",
        ));
    }

    // Check if session is in a valid state for uploading
    if !matches!(
        session.status,
        UploadSessionStatus::Pending | UploadSessionStatus::InProgress
    ) {
        return Err(ApiError::validation_failed(format!(
            "Cannot upload chunk: session status is {:?}",
            session.status
        )));
    }

    // Parse Content-Range header
    // Expected format: "bytes {start}-{end}/{total}"
    let content_range = headers
        .get("content-range")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::validation_failed("Missing Content-Range header"))?;

    let (start, end, total) = parse_content_range(content_range)?;

    // Validate range
    if total != session.file_size {
        return Err(ApiError::validation_failed(format!(
            "Content-Range total ({}) does not match file size ({})",
            total, session.file_size
        )));
    }

    if start != session.bytes_received {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "CHUNK_OFFSET_MISMATCH",
            format!(
                "Expected chunk at offset {}, got {}",
                session.bytes_received, start
            ),
        ));
    }

    let chunk_size = end - start + 1;

    // Create temp directory if it doesn't exist
    tokio::fs::create_dir_all(&session.temp_dir)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to create temp directory: {}", e)))?;

    // Determine chunk file name
    let chunk_file = format!("{}/chunk_{:04}", session.temp_dir, session.chunks_completed);

    // Stream body to chunk file
    let mut file = tokio::fs::File::create(&chunk_file)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to create chunk file: {}", e)))?;

    let mut body_stream = body.into_data_stream();
    let mut bytes_written = 0u64;

    while let Some(chunk_result) = body_stream.next().await {
        let chunk = chunk_result
            .map_err(|e| ApiError::internal(format!("Failed to read chunk data: {}", e)))?;

        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .map_err(|e| ApiError::internal(format!("Failed to write chunk: {}", e)))?;

        bytes_written += chunk.len() as u64;
    }

    // Verify chunk size
    if bytes_written != chunk_size {
        return Err(ApiError::validation_failed(format!(
            "Chunk size mismatch: expected {}, got {}",
            chunk_size, bytes_written
        )));
    }

    // Update session
    session.bytes_received += bytes_written;
    session.chunks_completed += 1;
    session.status = UploadSessionStatus::InProgress;
    session.updated_at = Utc::now();

    UPLOAD_STORE.put(session.clone()).await;

    tracing::debug!(
        upload_id = %upload_id,
        chunk = session.chunks_completed,
        bytes_received = session.bytes_received,
        "Chunk uploaded"
    );

    Ok(Json(ChunkUploadResponse {
        upload_id: upload_id.clone(),
        bytes_received: session.bytes_received,
        bytes_total: session.file_size,
        chunks_completed: session.chunks_completed,
        chunks_total: session.total_chunks,
        progress: session.bytes_received as f64 / session.file_size as f64,
    }))
}

/// Helper function to parse Content-Range header
/// Expected format: "bytes {start}-{end}/{total}"
pub(super) fn parse_content_range(content_range: &str) -> Result<(u64, u64, u64), ApiError> {
    let parts: Vec<&str> = content_range.split_whitespace().collect();
    if parts.len() != 2 || parts[0] != "bytes" {
        return Err(ApiError::validation_failed(
            "Invalid Content-Range header format",
        ));
    }

    let range_parts: Vec<&str> = parts[1].split('/').collect();
    if range_parts.len() != 2 {
        return Err(ApiError::validation_failed(
            "Invalid Content-Range header format",
        ));
    }

    let range: Vec<&str> = range_parts[0].split('-').collect();
    if range.len() != 2 {
        return Err(ApiError::validation_failed(
            "Invalid Content-Range header format",
        ));
    }

    let start = range[0]
        .parse::<u64>()
        .map_err(|_| ApiError::validation_failed("Invalid range start"))?;
    let end = range[1]
        .parse::<u64>()
        .map_err(|_| ApiError::validation_failed("Invalid range end"))?;
    let total = range_parts[1]
        .parse::<u64>()
        .map_err(|_| ApiError::validation_failed("Invalid total size"))?;

    Ok((start, end, total))
}
