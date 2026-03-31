// SPDX-License-Identifier: BSL-1.1

//! HTTP webhook and trigger invocation handlers
//!
//! Provides endpoints for invoking functions and flows via HTTP triggers.
//! Supports both nanoid-based secure webhooks and name-based trigger URLs.
//! Offers sync (wait for result) and async (fire-and-forget) execution modes.

mod config;
mod execution;
mod helpers;
mod lookup;
mod types;

pub use types::{InvokeQuery, WebhookResponse};

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, Method},
    Json,
};

use crate::{error::ApiError, state::AppState};

#[cfg(feature = "storage-rocksdb")]
use execution::invoke_http_trigger_internal;
#[cfg(feature = "storage-rocksdb")]
use types::TriggerLookup;

/// Invoke webhook by nanoid-based webhook_id (no path suffix)
#[cfg(feature = "storage-rocksdb")]
pub async fn invoke_webhook(
    State(state): State<AppState>,
    Path((repo, webhook_id)): Path<(String, String)>,
    method: Method,
    headers: HeaderMap,
    Query(query): Query<InvokeQuery>,
    body: Option<Json<serde_json::Value>>,
) -> Result<Json<WebhookResponse>, ApiError> {
    invoke_http_trigger_internal(
        &state,
        &repo,
        TriggerLookup::ByWebhookId(webhook_id),
        None,
        method,
        headers,
        query,
        body,
    )
    .await
}

/// Invoke webhook by webhook_id with path suffix
#[cfg(feature = "storage-rocksdb")]
pub async fn invoke_webhook_with_path(
    State(state): State<AppState>,
    Path((repo, webhook_id, path_suffix)): Path<(String, String, String)>,
    method: Method,
    headers: HeaderMap,
    Query(query): Query<InvokeQuery>,
    body: Option<Json<serde_json::Value>>,
) -> Result<Json<WebhookResponse>, ApiError> {
    invoke_http_trigger_internal(
        &state,
        &repo,
        TriggerLookup::ByWebhookId(webhook_id),
        Some(path_suffix),
        method,
        headers,
        query,
        body,
    )
    .await
}

/// Invoke trigger by unique name (no path suffix)
#[cfg(feature = "storage-rocksdb")]
pub async fn invoke_trigger(
    State(state): State<AppState>,
    Path((repo, trigger_name)): Path<(String, String)>,
    method: Method,
    headers: HeaderMap,
    Query(query): Query<InvokeQuery>,
    body: Option<Json<serde_json::Value>>,
) -> Result<Json<WebhookResponse>, ApiError> {
    invoke_http_trigger_internal(
        &state,
        &repo,
        TriggerLookup::ByName(trigger_name),
        None,
        method,
        headers,
        query,
        body,
    )
    .await
}

/// Invoke trigger by name with path suffix
#[cfg(feature = "storage-rocksdb")]
pub async fn invoke_trigger_with_path(
    State(state): State<AppState>,
    Path((repo, trigger_name, path_suffix)): Path<(String, String, String)>,
    method: Method,
    headers: HeaderMap,
    Query(query): Query<InvokeQuery>,
    body: Option<Json<serde_json::Value>>,
) -> Result<Json<WebhookResponse>, ApiError> {
    invoke_http_trigger_internal(
        &state,
        &repo,
        TriggerLookup::ByName(trigger_name),
        Some(path_suffix),
        method,
        headers,
        query,
        body,
    )
    .await
}

// Stub implementations when RocksDB is not available
#[cfg(not(feature = "storage-rocksdb"))]
pub async fn invoke_webhook(
    State(_state): State<AppState>,
    Path((_repo, _webhook_id)): Path<(String, String)>,
    _method: Method,
    _headers: HeaderMap,
    Query(_query): Query<InvokeQuery>,
    _body: Option<Json<serde_json::Value>>,
) -> Result<Json<WebhookResponse>, ApiError> {
    Err(ApiError::internal(
        "Webhooks require RocksDB storage backend",
    ))
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn invoke_webhook_with_path(
    State(_state): State<AppState>,
    Path((_repo, _webhook_id, _path_suffix)): Path<(String, String, String)>,
    _method: Method,
    _headers: HeaderMap,
    Query(_query): Query<InvokeQuery>,
    _body: Option<Json<serde_json::Value>>,
) -> Result<Json<WebhookResponse>, ApiError> {
    Err(ApiError::internal(
        "Webhooks require RocksDB storage backend",
    ))
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn invoke_trigger(
    State(_state): State<AppState>,
    Path((_repo, _trigger_name)): Path<(String, String)>,
    _method: Method,
    _headers: HeaderMap,
    Query(_query): Query<InvokeQuery>,
    _body: Option<Json<serde_json::Value>>,
) -> Result<Json<WebhookResponse>, ApiError> {
    Err(ApiError::internal(
        "Triggers require RocksDB storage backend",
    ))
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn invoke_trigger_with_path(
    State(_state): State<AppState>,
    Path((_repo, _trigger_name, _path_suffix)): Path<(String, String, String)>,
    _method: Method,
    _headers: HeaderMap,
    Query(_query): Query<InvokeQuery>,
    _body: Option<Json<serde_json::Value>>,
) -> Result<Json<WebhookResponse>, ApiError> {
    Err(ApiError::internal(
        "Triggers require RocksDB storage backend",
    ))
}
