// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Types for the Fetch API implementation

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Request body types supported by fetch
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum FetchBody {
    /// Plain text body
    Text(String),
    /// JSON body (will be serialized)
    Json(serde_json::Value),
    /// Form data (serialized entries)
    FormData(String),
    /// Binary data as base64
    ArrayBuffer(String),
}

/// Fetch request descriptor (sent from JS to Rust)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchRequest {
    /// The URL to fetch
    pub url: String,
    /// HTTP method (GET, POST, etc.)
    #[serde(default = "default_method")]
    pub method: String,
    /// Request headers
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Request body
    pub body: Option<FetchBody>,
    /// AbortController signal ID (links to AbortRegistry)
    pub signal_id: Option<String>,
    /// Request timeout in milliseconds
    pub timeout_ms: Option<u64>,
    /// Request mode (cors, no-cors, same-origin)
    #[serde(default = "default_mode")]
    pub mode: String,
    /// Credentials mode (omit, same-origin, include)
    #[serde(default = "default_credentials")]
    pub credentials: String,
    /// Cache mode
    #[serde(default = "default_cache")]
    pub cache: String,
    /// Redirect mode (follow, error, manual)
    #[serde(default = "default_redirect")]
    pub redirect: String,
}

fn default_method() -> String {
    "GET".to_string()
}

fn default_mode() -> String {
    "cors".to_string()
}

fn default_credentials() -> String {
    "same-origin".to_string()
}

fn default_cache() -> String {
    "default".to_string()
}

fn default_redirect() -> String {
    "follow".to_string()
}

/// Response metadata returned to JavaScript
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchResponseMeta {
    /// Unique stream ID for reading body chunks
    pub stream_id: String,
    /// HTTP status code
    pub status: u16,
    /// HTTP status text
    pub status_text: String,
    /// Response headers
    pub headers: HashMap<String, String>,
    /// Final URL (after redirects)
    pub url: String,
    /// Whether the response was redirected
    pub redirected: bool,
    /// Response type (basic, cors, error, opaque, opaqueredirect)
    #[serde(default = "default_response_type")]
    pub response_type: String,
}

fn default_response_type() -> String {
    "basic".to_string()
}

/// Error types for fetch operations
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "message")]
pub enum FetchError {
    /// Network error (connection failed, DNS error, etc.)
    Network(String),
    /// Request was aborted
    Abort(String),
    /// URL not allowed by network policy
    UrlNotAllowed(String),
    /// Request timeout
    Timeout,
    /// Invalid request (malformed URL, invalid method, etc.)
    TypeError(String),
}

impl FetchError {
    /// Convert to JSON for JavaScript
    #[allow(dead_code)]
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            FetchError::Network(msg) => serde_json::json!({
                "error": "NetworkError",
                "message": msg
            }),
            FetchError::Abort(msg) => serde_json::json!({
                "error": "AbortError",
                "message": msg
            }),
            FetchError::UrlNotAllowed(url) => serde_json::json!({
                "error": "TypeError",
                "message": format!("URL not allowed by network policy: {}", url)
            }),
            FetchError::Timeout => serde_json::json!({
                "error": "TimeoutError",
                "message": "The operation timed out"
            }),
            FetchError::TypeError(msg) => serde_json::json!({
                "error": "TypeError",
                "message": msg
            }),
        }
    }
}
