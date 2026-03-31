// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Helper functions for the QuickJS runtime.
//!
//! Provides JSON error formatting and async-to-sync bridging utilities.

/// Default timeout for external HTTP fetch requests (2 minutes).
pub(super) const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Creates a JSON error response with properly escaped error message.
///
/// This prevents JSON injection vulnerabilities when error messages contain
/// special characters like quotes.
pub(super) fn json_error(message: impl std::fmt::Display) -> String {
    serde_json::to_string(&serde_json::json!({ "error": message.to_string() }))
        .unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string())
}

/// Creates a JSON error response with additional fields.
pub(super) fn json_error_with_fields(
    message: impl std::fmt::Display,
    extra: serde_json::Value,
) -> String {
    let mut obj = serde_json::json!({ "error": message.to_string() });
    if let (Some(obj_map), Some(extra_map)) = (obj.as_object_mut(), extra.as_object()) {
        for (k, v) in extra_map {
            obj_map.insert(k.clone(), v.clone());
        }
    }
    serde_json::to_string(&obj)
        .unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string())
}

/// Run an async function in a blocking context.
///
/// This is used to bridge between the synchronous JS runtime and async Rust code.
/// Uses tokio's block_in_place to avoid blocking the async runtime.
pub(super) fn run_async_blocking<F, T>(future: F) -> T
where
    F: std::future::Future<Output = T>,
{
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(future))
}

/// Run an async function in a blocking context with a timeout.
///
/// Used for external HTTP fetch requests to prevent indefinite blocking when
/// the target service is unresponsive. Returns an error if the future does not
/// complete within the timeout duration.
pub(super) fn run_async_blocking_with_timeout<F, T>(
    future: F,
    timeout: std::time::Duration,
) -> std::result::Result<T, raisin_error::Error>
where
    F: std::future::Future<Output = T>,
{
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(async {
            tokio::time::timeout(timeout, future).await.map_err(|_| {
                raisin_error::Error::Internal(format!(
                    "HTTP request timed out after {}s",
                    timeout.as_secs()
                ))
            })
        })
    })
}
