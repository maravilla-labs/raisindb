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

//! Error types for the OAuth 2.1 authorization server.
//!
//! Variants map onto the registered OAuth error codes (RFC 6749 §5.2 and
//! §4.1.2.1, RFC 7591 §3.2.2) so the HTTP layer can render an RFC-compliant
//! error response without re-classifying the failure.

use serde::Serialize;
use thiserror::Error;

/// An error raised while servicing an OAuth 2.1 request.
///
/// Each variant carries the human-readable detail that becomes the
/// `error_description` field of the JSON error body. The [`AuthServerError::code`]
/// method yields the machine-readable `error` value.
#[derive(Debug, Error)]
pub enum AuthServerError {
    /// The request is missing a parameter, repeats one, or is otherwise malformed.
    #[error("invalid_request: {0}")]
    InvalidRequest(String),

    /// Client authentication failed or the client is unknown.
    #[error("invalid_client: {0}")]
    InvalidClient(String),

    /// The authorization grant (code) is invalid, expired, revoked, or reused.
    #[error("invalid_grant: {0}")]
    InvalidGrant(String),

    /// The grant type is not supported by the token endpoint.
    #[error("unsupported_grant_type: {0}")]
    UnsupportedGrantType(String),

    /// The requested response type is not supported by the authorization endpoint.
    #[error("unsupported_response_type: {0}")]
    UnsupportedResponseType(String),

    /// The requested scope is invalid, unknown, or exceeds what the client may ask for.
    #[error("invalid_scope: {0}")]
    InvalidScope(String),

    /// The client is not allowed to use this grant or endpoint.
    #[error("unauthorized_client: {0}")]
    UnauthorizedClient(String),

    /// The redirect URI is missing or does not match a registered value.
    ///
    /// Distinguished from [`AuthServerError::InvalidRequest`] because the HTTP
    /// layer MUST NOT redirect back to an unvalidated URI (RFC 6749 §4.1.2.1).
    #[error("invalid_redirect_uri: {0}")]
    InvalidRedirectUri(String),

    /// The dynamic client registration request is not acceptable (RFC 7591 §3.2.2).
    #[error("invalid_client_metadata: {0}")]
    InvalidClientMetadata(String),

    /// The resource owner could not be authenticated for the authorization request.
    #[error("access_denied: {0}")]
    AccessDenied(String),

    /// An unexpected internal failure (storage, signing, ...).
    #[error("server_error: {0}")]
    ServerError(String),
}

impl AuthServerError {
    /// The registered OAuth `error` code for this failure.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest(_) => "invalid_request",
            Self::InvalidClient(_) => "invalid_client",
            Self::InvalidGrant(_) => "invalid_grant",
            Self::UnsupportedGrantType(_) => "unsupported_grant_type",
            Self::UnsupportedResponseType(_) => "unsupported_response_type",
            Self::InvalidScope(_) => "invalid_scope",
            Self::UnauthorizedClient(_) => "unauthorized_client",
            // RFC 6749 does not define a distinct redirect-uri code; the
            // canonical mapping for a bad/unknown redirect URI is invalid_request.
            Self::InvalidRedirectUri(_) => "invalid_request",
            Self::InvalidClientMetadata(_) => "invalid_client_metadata",
            Self::AccessDenied(_) => "access_denied",
            Self::ServerError(_) => "server_error",
        }
    }

    /// The human-readable detail describing this failure.
    pub fn description(&self) -> String {
        match self {
            Self::InvalidRequest(m)
            | Self::InvalidClient(m)
            | Self::InvalidGrant(m)
            | Self::UnsupportedGrantType(m)
            | Self::UnsupportedResponseType(m)
            | Self::InvalidScope(m)
            | Self::UnauthorizedClient(m)
            | Self::InvalidRedirectUri(m)
            | Self::InvalidClientMetadata(m)
            | Self::AccessDenied(m)
            | Self::ServerError(m) => m.clone(),
        }
    }

    /// Whether this error is safe to redirect back to the client.
    ///
    /// Errors discovered before the `redirect_uri` is validated (an unknown
    /// client or an unregistered redirect URI) MUST be shown directly to the
    /// resource owner rather than redirected (RFC 6749 §4.1.2.1).
    pub fn is_redirectable(&self) -> bool {
        !matches!(self, Self::InvalidClient(_) | Self::InvalidRedirectUri(_))
    }

    /// Render this error as the RFC 6749 §5.2 JSON body.
    pub fn to_response_body(&self) -> OAuthErrorBody {
        OAuthErrorBody {
            error: self.code().to_string(),
            error_description: self.description(),
        }
    }
}

/// The RFC 6749 §5.2 / RFC 7591 §3.2.2 error response body.
#[derive(Debug, Clone, Serialize)]
pub struct OAuthErrorBody {
    /// The registered OAuth error code.
    pub error: String,
    /// A human-readable description of the failure.
    pub error_description: String,
}

/// Convenience alias for fallible authorization-server operations.
pub type AuthServerResult<T> = Result<T, AuthServerError>;
