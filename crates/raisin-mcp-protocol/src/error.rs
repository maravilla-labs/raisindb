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

//! Error types for the MCP server surface.
//!
//! [`McpError`] folds the shared [`raisin_error::Error`] in via `#[from]`, so
//! any call into the RaisinDB core / functions layer propagates with `?`
//! directly. JSON-RPC dispatch maps each variant onto a numeric error code
//! through [`McpError::code`].

use thiserror::Error;

/// Result alias for MCP operations.
pub type Result<T> = std::result::Result<T, McpError>;

/// Errors raised by the MCP transport and tool-dispatch layers.
#[derive(Debug, Error)]
pub enum McpError {
    /// Malformed, unsupported, or out-of-order MCP request.
    #[error("Protocol error: {0}")]
    Protocol(String),

    /// Failure to decode a JSON-RPC envelope or tool argument payload.
    #[error("Parse error: {0}")]
    Parse(String),

    /// A referenced method, tool, or resource is not registered.
    #[error("Not found: {0}")]
    NotFound(String),

    /// Arguments supplied to a tool call failed validation.
    #[error("Invalid params: {0}")]
    InvalidParams(String),

    /// The caller is not permitted to invoke the requested capability.
    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    /// A function tool ran but reported a domain-level failure.
    #[error("Function failed: {0}")]
    FunctionFailed(String),

    /// The request's protocol version is unknown or unsupported.
    ///
    /// Carries the versions this server does support, which the error's `data`
    /// member reports alongside the requested one so the client can pick a
    /// mutually supported revision and retry.
    #[error("Unsupported protocol version: {requested}")]
    UnsupportedProtocolVersion {
        /// The version the client asked for.
        requested: String,
    },

    /// A required per-request client capability was not declared.
    #[error("Missing required client capability: {0}")]
    MissingClientCapability(String),

    /// HTTP headers disagree with the request body, or a required header is
    /// missing or malformed. The transport answers `400` for this.
    #[error("Header mismatch: {0}")]
    HeaderMismatch(String),

    /// Underlying RaisinDB error (storage, validation, auth, functions, ...).
    #[error(transparent)]
    Raisin(#[from] raisin_error::Error),

    /// JSON (de)serialization failure outside of request parsing.
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

impl McpError {
    /// Construct a protocol error.
    pub fn protocol(msg: impl Into<String>) -> Self {
        Self::Protocol(msg.into())
    }

    /// Construct a parse error.
    pub fn parse(msg: impl Into<String>) -> Self {
        Self::Parse(msg.into())
    }

    /// Construct a not-found error.
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::NotFound(msg.into())
    }

    /// Construct an invalid-params error.
    pub fn invalid_params(msg: impl Into<String>) -> Self {
        Self::InvalidParams(msg.into())
    }

    /// Construct an unauthorized error.
    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self::Unauthorized(msg.into())
    }

    /// Construct a function-failure error.
    pub fn function_failed(msg: impl Into<String>) -> Self {
        Self::FunctionFailed(msg.into())
    }

    /// Construct an unsupported-protocol-version error.
    pub fn unsupported_protocol_version(requested: impl Into<String>) -> Self {
        Self::UnsupportedProtocolVersion {
            requested: requested.into(),
        }
    }

    /// Construct a missing-client-capability error.
    pub fn missing_client_capability(msg: impl Into<String>) -> Self {
        Self::MissingClientCapability(msg.into())
    }

    /// Construct a header-mismatch error.
    pub fn header_mismatch(msg: impl Into<String>) -> Self {
        Self::HeaderMismatch(msg.into())
    }

    /// Map this error onto the JSON-RPC 2.0 error code used on the wire.
    ///
    /// Codes follow the JSON-RPC reserved range where applicable (`-32700`
    /// parse, `-32600` invalid request, `-32601` method not found, `-32602`
    /// invalid params).
    ///
    /// MCP partitions the implementation-defined range: `-32000..-32019` is
    /// ours to use, while `-32020..-32099` is reserved for codes the spec
    /// itself defines — `-32020` header mismatch, `-32021` missing client
    /// capability, `-32022` unsupported protocol version. Codes from earlier
    /// revisions are reserved and never reused, which is why `FunctionFailed`
    /// no longer maps to `-32002`: that was "resource not found" up to
    /// 2025-11-25 (now `-32602`), and reusing it would make a tool's own
    /// failure indistinguishable from a missing resource to any client that
    /// still knows the old meaning.
    pub fn code(&self) -> i32 {
        match self {
            Self::Parse(_) => -32700,
            Self::Protocol(_) => -32600,
            Self::NotFound(_) => -32601,
            Self::InvalidParams(_) => -32602,
            Self::Unauthorized(_) => -32001,
            Self::FunctionFailed(_) => -32003,
            Self::HeaderMismatch(_) => -32020,
            Self::MissingClientCapability(_) => -32021,
            Self::UnsupportedProtocolVersion { .. } => -32022,
            Self::Raisin(_) => -32000,
            Self::Serialization(_) => -32603,
        }
    }

    /// The `data` member the spec requires for certain error codes.
    ///
    /// `-32022` MUST carry `{ supported, requested }` so the client can choose
    /// a mutually supported revision and retry; `-32021` MUST carry
    /// `{ requiredCapabilities }`. Returning `None` leaves `data` off entirely,
    /// which is correct for every other variant.
    pub fn data(&self) -> Option<serde_json::Value> {
        match self {
            Self::UnsupportedProtocolVersion { requested } => Some(serde_json::json!({
                "supported": crate::protocol::SUPPORTED_PROTOCOL_VERSIONS,
                "requested": requested,
            })),
            Self::MissingClientCapability(capability) => Some(serde_json::json!({
                "requiredCapabilities": { capability.clone(): {} },
            })),
            _ => None,
        }
    }
}
