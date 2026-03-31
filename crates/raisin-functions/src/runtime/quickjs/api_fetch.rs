// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! W3C Fetch API registration for the QuickJS runtime.
//!
//! Registers internal functions for the W3C Fetch API polyfill:
//! fetch_request, abort controller, and stream operations.

use rquickjs::{Ctx, Function, Object};
use std::sync::Arc;

use super::helpers::{run_async_blocking_with_timeout, FETCH_TIMEOUT};
use crate::api::FunctionApi;
use crate::runtime::fetch::{AbortRegistry, FetchRequest, StreamRegistry};

/// Register W3C Fetch API internal functions.
///
/// These functions are wrapped by the JavaScript polyfill to provide
/// the standard fetch(), Request, Response, Headers, ReadableStream,
/// AbortController, and FormData APIs.
pub(super) fn register_fetch_internal<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
    api: Arc<dyn FunctionApi>,
    abort_registry: Arc<AbortRegistry>,
    stream_registry: Arc<StreamRegistry>,
) -> std::result::Result<(), rquickjs::Error> {
    register_fetch_request(ctx, internal, api, &abort_registry, &stream_registry)?;
    register_abort_ops(ctx, internal, &abort_registry)?;
    register_stream_ops(ctx, internal, &stream_registry)?;

    Ok(())
}

/// Register the main fetch_request function.
fn register_fetch_request<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
    api: Arc<dyn FunctionApi>,
    abort_registry: &Arc<AbortRegistry>,
    stream_registry: &Arc<StreamRegistry>,
) -> std::result::Result<(), rquickjs::Error> {
    let api_fetch = api.clone();
    let abort_reg = abort_registry.clone();
    let stream_reg = stream_registry.clone();
    let fetch_request_fn = Function::new(ctx.clone(), move |request_json: String| {
        let api = api_fetch.clone();
        let abort_reg = abort_reg.clone();
        let stream_reg = stream_reg.clone();

        let request: FetchRequest = match serde_json::from_str(&request_json) {
            Ok(r) => r,
            Err(e) => {
                return serde_json::json!({
                    "error": "TypeError",
                    "message": format!("Invalid request: {}", e)
                })
                .to_string();
            }
        };

        // Check if already aborted
        if let Some(ref signal_id) = request.signal_id {
            if abort_reg.is_aborted(signal_id) {
                return serde_json::json!({
                    "error": "AbortError",
                    "message": "The operation was aborted."
                })
                .to_string();
            }
        }

        let url = request.url.clone();
        let method = request.method.clone();
        let signal_id = request.signal_id.clone();

        // Build options for existing http_request
        let mut opts = serde_json::json!({
            "method": method,
            "headers": request.headers,
        });

        // Add body if present
        if let Some(body) = &request.body {
            match body {
                crate::runtime::fetch::FetchBody::Text(s) => {
                    opts["body"] = serde_json::Value::String(s.clone());
                }
                crate::runtime::fetch::FetchBody::Json(v) => {
                    opts["body"] = v.clone();
                }
                crate::runtime::fetch::FetchBody::FormData(s) => {
                    opts["body"] = serde_json::Value::String(s.clone());
                }
                crate::runtime::fetch::FetchBody::ArrayBuffer(s) => {
                    opts["body"] = serde_json::Value::String(s.clone());
                }
            }
        }

        // Add timeout if specified
        if let Some(timeout) = request.timeout_ms {
            opts["timeout"] = serde_json::Value::Number(timeout.into());
        }

        // Execute the request with timeout to prevent indefinite blocking
        let result = run_async_blocking_with_timeout(
            async move {
                // Check abort before starting
                if let Some(ref signal_id) = signal_id {
                    if abort_reg.is_aborted(signal_id) {
                        return Err(raisin_error::Error::Validation(
                            "The operation was aborted.".to_string(),
                        ));
                    }
                }

                // Make the HTTP request using the existing API
                let response = api.http_request(&method, &url, opts).await?;

                tracing::trace!(
                    response = %serde_json::to_string(&response).unwrap_or_else(|_| "<failed to serialize>".into()),
                    "fetch_request - response from http_request"
                );

                // Extract response data
                let status = response
                    .get("status")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u16;

                let headers: std::collections::HashMap<String, String> = response
                    .get("headers")
                    .and_then(|v| v.as_object())
                    .map(|obj| {
                        obj.iter()
                            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                            .collect()
                    })
                    .unwrap_or_default();

                // Get body and create a stream
                let body = response.get("body").cloned().unwrap_or(serde_json::Value::Null);
                let body_type = match &body {
                    serde_json::Value::Null => "null",
                    serde_json::Value::String(_) => "string",
                    serde_json::Value::Object(_) => "object",
                    serde_json::Value::Array(_) => "array",
                    serde_json::Value::Number(_) => "number",
                    serde_json::Value::Bool(_) => "bool",
                };
                tracing::trace!(
                    body_type = body_type,
                    is_null = body.is_null(),
                    is_string = body.is_string(),
                    len_if_string = body.as_str().map(|s| s.len()).unwrap_or(0),
                    "fetch_request - extracted body"
                );
                let body_bytes = match body {
                    serde_json::Value::String(s) => bytes::Bytes::from(s),
                    serde_json::Value::Null => bytes::Bytes::new(),
                    other => bytes::Bytes::from(serde_json::to_string(&other).unwrap_or_default()),
                };

                tracing::trace!(body_bytes_len = body_bytes.len(), "fetch_request - body_bytes");

                // Create a buffered stream from the body
                let stream_id = stream_reg.start_buffered_stream(body_bytes.clone());

                tracing::trace!(
                    stream_id = %stream_id,
                    body_preview = %String::from_utf8_lossy(&body_bytes[..std::cmp::min(100, body_bytes.len())]),
                    "fetch_request - created stream"
                );

                Ok(serde_json::json!({
                    "stream_id": stream_id,
                    "status": status,
                    "status_text": get_status_text(status),
                    "headers": headers,
                    "url": url,
                    "redirected": false,
                    "response_type": "basic"
                }))
            },
            FETCH_TIMEOUT,
        )
        .and_then(|r| r);

        match result {
            Ok(v) => v.to_string(),
            Err(e) => {
                let error_str = e.to_string();
                if error_str.contains("aborted") {
                    serde_json::json!({
                        "error": "AbortError",
                        "message": "The operation was aborted."
                    })
                    .to_string()
                } else if error_str.contains("not allowed") {
                    serde_json::json!({
                        "error": "TypeError",
                        "message": error_str
                    })
                    .to_string()
                } else {
                    serde_json::json!({
                        "error": "NetworkError",
                        "message": error_str
                    })
                    .to_string()
                }
            }
        }
    })?;
    internal.set("fetch_request", fetch_request_fn)?;

    Ok(())
}

/// Register abort controller operations (create, abort, is_aborted).
fn register_abort_ops<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
    abort_registry: &Arc<AbortRegistry>,
) -> std::result::Result<(), rquickjs::Error> {
    // fetch_create_abort_controller
    let abort_reg_create = abort_registry.clone();
    let create_abort_fn = Function::new(ctx.clone(), move || -> String {
        abort_reg_create.create_controller()
    })?;
    internal.set("fetch_create_abort_controller", create_abort_fn)?;

    // fetch_abort
    let abort_reg_abort = abort_registry.clone();
    let abort_fn = Function::new(
        ctx.clone(),
        move |id: String, reason: Option<String>| -> bool { abort_reg_abort.abort(&id, reason) },
    )?;
    internal.set("fetch_abort", abort_fn)?;

    // fetch_is_aborted
    let abort_reg_check = abort_registry.clone();
    let is_aborted_fn = Function::new(ctx.clone(), move |id: String| -> bool {
        abort_reg_check.is_aborted(&id)
    })?;
    internal.set("fetch_is_aborted", is_aborted_fn)?;

    Ok(())
}

/// Register stream operations (read, lock, unlock, cancel).
fn register_stream_ops<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
    stream_registry: &Arc<StreamRegistry>,
) -> std::result::Result<(), rquickjs::Error> {
    use super::helpers::run_async_blocking;

    // fetch_stream_read
    let stream_reg_read = stream_registry.clone();
    let stream_read_fn = Function::new(ctx.clone(), move |stream_id: String| -> String {
        let stream_reg = stream_reg_read.clone();
        let result = run_async_blocking(async move { stream_reg.read_chunk(&stream_id).await });
        result.to_json()
    })?;
    internal.set("fetch_stream_read", stream_read_fn)?;

    // fetch_stream_lock
    let stream_reg_lock = stream_registry.clone();
    let stream_lock_fn = Function::new(ctx.clone(), move |stream_id: String| -> bool {
        stream_reg_lock.lock(&stream_id)
    })?;
    internal.set("fetch_stream_lock", stream_lock_fn)?;

    // fetch_stream_unlock
    let stream_reg_unlock = stream_registry.clone();
    let stream_unlock_fn = Function::new(ctx.clone(), move |stream_id: String| -> bool {
        stream_reg_unlock.unlock(&stream_id)
    })?;
    internal.set("fetch_stream_unlock", stream_unlock_fn)?;

    // fetch_stream_cancel
    let stream_reg_cancel = stream_registry.clone();
    let stream_cancel_fn = Function::new(ctx.clone(), move |stream_id: String| -> bool {
        stream_reg_cancel.cancel(&stream_id)
    })?;
    internal.set("fetch_stream_cancel", stream_cancel_fn)?;

    Ok(())
}

/// Get HTTP status text for a status code.
fn get_status_text(status: u16) -> &'static str {
    match status {
        100 => "Continue",
        101 => "Switching Protocols",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "",
    }
}
