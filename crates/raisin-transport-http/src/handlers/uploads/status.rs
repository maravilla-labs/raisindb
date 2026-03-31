// SPDX-License-Identifier: BSL-1.1

//! Upload status and progress query handlers.

use axum::{body::Body, extract::Path, http::StatusCode, response::Response, Json};

use crate::error::ApiError;

use super::types::{UploadSession, UPLOAD_STORE};

/// HEAD /api/uploads/{id} - Get upload progress
pub async fn get_upload_progress(Path(upload_id): Path<String>) -> Result<Response, ApiError> {
    let session = UPLOAD_STORE
        .get(&upload_id)
        .await
        .ok_or_else(|| ApiError::not_found("Upload session not found"))?;

    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::OK;

    let headers = response.headers_mut();
    headers.insert(
        "Upload-Offset",
        session
            .bytes_received
            .to_string()
            .parse()
            .expect("numeric string is valid header value"),
    );
    headers.insert(
        "Upload-Length",
        session
            .file_size
            .to_string()
            .parse()
            .expect("numeric string is valid header value"),
    );
    headers.insert(
        "Upload-Status",
        format!("{:?}", session.status)
            .to_lowercase()
            .parse()
            .expect("status debug string is valid header value"),
    );

    Ok(response)
}

/// GET /api/uploads/{id} - Get upload status
pub async fn get_upload_status(
    Path(upload_id): Path<String>,
) -> Result<Json<UploadSession>, ApiError> {
    let session = UPLOAD_STORE
        .get(&upload_id)
        .await
        .ok_or_else(|| ApiError::not_found("Upload session not found"))?;

    Ok(Json(session))
}
