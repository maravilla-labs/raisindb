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

//! Failures talking to a remote MCP server.
//!
//! Deliberately the same five-way classification as
//! `virtual_mount_sync::adapter::AdapterError`, and for the same reason: one
//! place decides whether a failure is worth retrying, so the caller, the
//! connection's health record and the job-retry decision can never disagree
//! about what an error meant. The reserved `code` strings are shared with the
//! connector stack so the admin console renders both with one vocabulary.

use thiserror::Error;

/// A failure invoking a remote MCP server.
#[derive(Debug, Error)]
pub enum RemoteToolError {
    /// HTTP 401/403, or a JSON-RPC auth error. The connection needs re-auth;
    /// its health flips to `auth_expired` and calls stop until an operator acts.
    #[error("mcp auth expired: {0}")]
    AuthExpired(String),

    /// HTTP 429. `retry_after_secs` carries the server's `Retry-After` when it
    /// sent one — honour it rather than guessing, or a shared remote gets a
    /// thundering herd from every RaisinDB node at once.
    #[error("mcp rate limited")]
    RateLimited {
        /// Seconds the server asked us to wait, when it said.
        retry_after_secs: Option<u64>,
    },

    /// The request can NEVER succeed as written: an endpoint 404, JSON-RPC
    /// `-32601` (method not found) / `-32602` (invalid params), or an unknown
    /// tool name.
    ///
    /// Distinct from [`Self::Transient`] because retrying is not merely useless,
    /// it is harmful — the connector stack learned this when a malformed remote
    /// root retried three times per job on every scheduler tick and got the
    /// OAuth app throttled.
    #[error("mcp configuration error: {0}")]
    Config(String),

    /// The peer is not speaking a protocol revision we can agree on.
    #[error("mcp protocol error: {0}")]
    Protocol(String),

    /// The server dropped the session we were using (a 404 answering a request
    /// that carried an `Mcp-Session-Id`).
    ///
    /// A distinct variant rather than a `Protocol` message, because the session
    /// layer must recognise it exactly to re-initialize and replay once. Matching
    /// on an error *string* to drive control flow is how that recovery silently
    /// stops working the first time someone rewords the message.
    #[error("mcp session expired")]
    SessionExpired,

    /// Anything else — connect failures, timeouts, 5xx, malformed JSON.
    #[error("mcp transient error: {0}")]
    Transient(String),
}

impl RemoteToolError {
    /// Classify an HTTP status into a typed error.
    pub fn from_status(status: u16, body: &str, retry_after_secs: Option<u64>) -> Self {
        let detail = truncate(body, 512);
        match status {
            401 | 403 => Self::AuthExpired(detail),
            404 => Self::Config(format!("endpoint not found (404): {detail}")),
            429 => Self::RateLimited { retry_after_secs },
            s if (400..500).contains(&s) => Self::Config(format!("http {s}: {detail}")),
            s => Self::Transient(format!("http {s}: {detail}")),
        }
    }

    /// Classify a JSON-RPC error code returned by the peer.
    ///
    /// `-32001` is not a spec code; it is what several servers use for
    /// "unauthorized", so it is matched best-effort alongside the standard ones.
    pub fn from_jsonrpc(code: i32, message: &str) -> Self {
        match code {
            -32601 => Self::Config(format!("method not found: {message}")),
            -32602 => Self::Config(format!("invalid params: {message}")),
            -32700 | -32600 => Self::Protocol(message.to_string()),
            -32001 => Self::AuthExpired(message.to_string()),
            _ => Self::Transient(format!("jsonrpc {code}: {message}")),
        }
    }

    /// Whether re-running the identical request could plausibly succeed.
    ///
    /// The single place this judgement is made. Note that a `true` here does NOT
    /// authorize an automatic retry of `tools/call` — see [`crate::client`] for
    /// why tool invocation is deliberately not retried.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::RateLimited { .. } | Self::Transient(_) => true,
            // AuthExpired needs a reconnect; Config and Protocol need an
            // operator edit. Retrying any of them just burns the remote's quota.
            // SessionExpired is handled by the session layer's replay and must
            // never reach a generic retry — it is false here so that if it ever
            // does leak out, it stops rather than loops.
            Self::AuthExpired(_) | Self::Config(_) | Self::Protocol(_) | Self::SessionExpired => {
                false
            }
        }
    }

    /// Stable machine-readable code, shared vocabulary with `AdapterError` so
    /// the admin console renders connector and MCP failures identically.
    pub fn code(&self) -> &'static str {
        match self {
            Self::AuthExpired(_) => "auth_expired",
            Self::RateLimited { .. } => "rate_limited",
            Self::Config(_) => "config_error",
            Self::Protocol(_) => "protocol_error",
            Self::SessionExpired => "session_expired",
            Self::Transient(_) => "transient_error",
        }
    }
}

impl From<reqwest::Error> for RemoteToolError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            Self::Transient(format!("request timed out: {err}"))
        } else if err.is_decode() {
            Self::Transient(format!("malformed response: {err}"))
        } else {
            Self::Transient(err.to_string())
        }
    }
}

/// Bound an untrusted remote body before it reaches a log line or an error
/// stored on a node property.
fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… ({} bytes)", &text[..end], text.len())
}

/// Result alias for client operations.
pub type Result<T> = std::result::Result<T, RemoteToolError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_matches_retryability() {
        assert!(!RemoteToolError::from_status(401, "nope", None).is_retryable());
        assert!(!RemoteToolError::from_status(404, "nope", None).is_retryable());
        assert!(!RemoteToolError::from_status(400, "bad", None).is_retryable());
        assert!(RemoteToolError::from_status(429, "slow", Some(30)).is_retryable());
        assert!(RemoteToolError::from_status(503, "later", None).is_retryable());
    }

    #[test]
    fn retry_after_is_preserved() {
        match RemoteToolError::from_status(429, "", Some(42)) {
            RemoteToolError::RateLimited { retry_after_secs } => {
                assert_eq!(retry_after_secs, Some(42))
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    #[test]
    fn jsonrpc_codes_are_not_retried_when_terminal() {
        assert!(!RemoteToolError::from_jsonrpc(-32601, "nope").is_retryable());
        assert!(!RemoteToolError::from_jsonrpc(-32602, "bad args").is_retryable());
        assert_eq!(
            RemoteToolError::from_jsonrpc(-32001, "unauthorized").code(),
            "auth_expired"
        );
    }

    #[test]
    fn truncation_respects_char_boundaries() {
        let body = "é".repeat(400); // 800 bytes
        let err = RemoteToolError::from_status(500, &body, None);
        // Must not panic, and must report the original size.
        assert!(err.to_string().contains("800 bytes"));
    }
}
