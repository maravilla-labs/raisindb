// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! HTTP operation callbacks for function execution.
//!
//! These callbacks implement the `raisin.http.*` API available to JavaScript functions.

use std::sync::Arc;

use serde_json::Value;

use crate::api::HttpRequestCallback;

/// Create http_request callback: `raisin.http.fetch(method, url, options)`
pub fn create_http_request(http_client: reqwest::Client) -> HttpRequestCallback {
    Arc::new(move |method: String, url: String, options: Value| {
        let client = http_client.clone();

        Box::pin(async move {
            // Parse options
            let headers = options.get("headers").and_then(|v| v.as_object()).cloned();
            let body = options.get("body").cloned();
            let timeout_ms = options
                .get("timeout")
                .and_then(|v| v.as_u64())
                .unwrap_or(30_000);

            // Build request
            let mut request = match method.to_uppercase().as_str() {
                "GET" => client.get(&url),
                "POST" => client.post(&url),
                "PUT" => client.put(&url),
                "DELETE" => client.delete(&url),
                "PATCH" => client.patch(&url),
                "HEAD" => client.head(&url),
                _ => {
                    return Err(raisin_error::Error::Validation(format!(
                        "Unsupported HTTP method: {}",
                        method
                    )))
                }
            };

            // Set timeout
            request = request.timeout(std::time::Duration::from_millis(timeout_ms));

            // Add headers
            if let Some(hdrs) = headers {
                for (key, value) in hdrs {
                    if let Some(val_str) = value.as_str() {
                        request = request.header(&key, val_str);
                    }
                }
            }

            // Add body
            if let Some(body_val) = body {
                if let Some(body_str) = body_val.as_str() {
                    request = request.body(body_str.to_string());
                } else {
                    // JSON body
                    request = request.json(&body_val);
                }
            }

            // Execute request
            let response = request
                .send()
                .await
                .map_err(|e| raisin_error::Error::Backend(format!("HTTP request failed: {}", e)))?;

            // Build response object
            let status = response.status().as_u16();
            let headers: serde_json::Map<String, Value> = response
                .headers()
                .iter()
                .map(|(k, v)| {
                    (
                        k.to_string(),
                        Value::String(v.to_str().unwrap_or("").to_string()),
                    )
                })
                .collect();

            let body_text = response.text().await.map_err(|e| {
                raisin_error::Error::Backend(format!("Failed to read response body: {}", e))
            })?;

            // Try to parse body as JSON, fall back to string
            let body_value =
                serde_json::from_str::<Value>(&body_text).unwrap_or(Value::String(body_text));

            Ok(serde_json::json!({
                "status": status,
                "headers": headers,
                "body": body_value
            }))
        })
    })
}
