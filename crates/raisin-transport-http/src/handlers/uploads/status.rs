// SPDX-License-Identifier: BSL-1.1

//! Upload status and progress query handlers.

use axum::{
    body::Body,
    extract::{Extension, Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    Json,
};

use raisin_models::auth::AuthContext;

use crate::{error::ApiError, middleware::TenantInfo, state::AppState};

use super::token::{principal, require_session_access, Principal};

use super::types::{UploadSession, UPLOAD_STORE};

/// HEAD /api/uploads/{id} - Get upload progress
pub async fn get_upload_progress(
    State(state): State<AppState>,
    Path(upload_id): Path<String>,
    Extension(tenant_info): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let session = UPLOAD_STORE
        .get(&upload_id)
        .await
        .ok_or_else(|| ApiError::not_found("Upload session not found"))?;
    let principal = principal(&state, &headers, &tenant_info, auth)?;
    require_session_access(&principal, &tenant_info, &session)?;

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
    State(state): State<AppState>,
    Path(upload_id): Path<String>,
    Extension(tenant_info): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    headers: HeaderMap,
) -> Result<Json<UploadSession>, ApiError> {
    let session = UPLOAD_STORE
        .get(&upload_id)
        .await
        .ok_or_else(|| ApiError::not_found("Upload session not found"))?;
    let principal = principal(&state, &headers, &tenant_info, auth)?;
    require_session_access(&principal, &tenant_info, &session)?;

    let mut session = session;
    if matches!(principal, Principal::Token(_)) {
        // A device holding a token has no business with server-side paths.
        session.temp_dir.clear();
        session.owner.clear();
        session.metadata = serde_json::Value::Null;
    }
    Ok(Json(session))
}
