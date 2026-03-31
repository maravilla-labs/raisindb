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

//! Error classification and handling for flow steps
//!
//! Classifies errors as retryable or non-retryable for proper handling.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Error that can occur during step execution
#[derive(Debug, Error)]
pub enum StepError {
    /// Retryable error (transient failure)
    #[error("Retryable error: {message}")]
    Retryable {
        /// Error message
        message: String,
        /// Optional source error
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// Non-retryable error (permanent failure)
    #[error("Non-retryable error: {message}")]
    NonRetryable {
        /// Error message
        message: String,
        /// Optional source error
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// Timeout error
    #[error("Step timed out after {timeout_ms}ms")]
    Timeout {
        /// Timeout duration in milliseconds
        timeout_ms: u64,
    },

    /// Cancelled by user or system
    #[error("Step cancelled")]
    Cancelled,

    /// Validation error (bad input)
    #[error("Validation error: {message}")]
    Validation {
        /// Validation error message
        message: String,
    },
}

impl StepError {
    /// Check if this error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            StepError::Retryable { .. } | StepError::Timeout { .. }
        )
    }

    /// Create a retryable error
    pub fn retryable(message: impl Into<String>) -> Self {
        Self::Retryable {
            message: message.into(),
            source: None,
        }
    }

    /// Create a non-retryable error
    pub fn non_retryable(message: impl Into<String>) -> Self {
        Self::NonRetryable {
            message: message.into(),
            source: None,
        }
    }

    /// Create a timeout error
    pub fn timeout(timeout_ms: u64) -> Self {
        Self::Timeout { timeout_ms }
    }

    /// Create a validation error
    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation {
            message: message.into(),
        }
    }
}

/// Error classification for common error types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorClass {
    /// Network/connectivity issues (retryable)
    Network,
    /// Rate limiting (retryable with backoff)
    RateLimit,
    /// Service unavailable (retryable)
    ServiceUnavailable,
    /// Authentication failed (non-retryable)
    Authentication,
    /// Authorization failed (non-retryable)
    Authorization,
    /// Bad request/validation (non-retryable)
    Validation,
    /// Resource not found (non-retryable)
    NotFound,
    /// Internal error (may be retryable)
    Internal,
    /// Unknown error
    Unknown,
}

impl ErrorClass {
    /// Check if this error class is typically retryable
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            ErrorClass::Network
                | ErrorClass::RateLimit
                | ErrorClass::ServiceUnavailable
                | ErrorClass::Internal
        )
    }

    /// Classify an HTTP status code
    pub fn from_http_status(status: u16) -> Self {
        match status {
            401 => ErrorClass::Authentication,
            403 => ErrorClass::Authorization,
            404 => ErrorClass::NotFound,
            400 | 422 => ErrorClass::Validation,
            429 => ErrorClass::RateLimit,
            500 | 502 | 504 => ErrorClass::Internal,
            503 => ErrorClass::ServiceUnavailable,
            _ if status >= 500 => ErrorClass::Internal,
            _ => ErrorClass::Unknown,
        }
    }

    /// Classify an error message (heuristic)
    pub fn from_error_message(message: &str) -> Self {
        let lower = message.to_lowercase();

        if lower.contains("timeout") || lower.contains("timed out") {
            return ErrorClass::Network;
        }
        if lower.contains("connection") || lower.contains("network") {
            return ErrorClass::Network;
        }
        if lower.contains("rate limit") || lower.contains("too many requests") {
            return ErrorClass::RateLimit;
        }
        if lower.contains("unauthorized") || lower.contains("authentication") {
            return ErrorClass::Authentication;
        }
        if lower.contains("forbidden") || lower.contains("permission") {
            return ErrorClass::Authorization;
        }
        if lower.contains("not found") {
            return ErrorClass::NotFound;
        }
        if lower.contains("invalid") || lower.contains("validation") {
            return ErrorClass::Validation;
        }
        if lower.contains("unavailable") || lower.contains("service") {
            return ErrorClass::ServiceUnavailable;
        }

        ErrorClass::Unknown
    }
}

/// Step error behavior configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OnErrorBehavior {
    /// Stop the flow on error (default)
    #[default]
    Stop,
    /// Skip this step and continue
    Skip,
    /// Continue to next step (ignore error)
    Continue,
    /// Trigger compensation/rollback
    Rollback,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_classification_retryable() {
        assert!(ErrorClass::Network.is_retryable());
        assert!(ErrorClass::RateLimit.is_retryable());
        assert!(ErrorClass::ServiceUnavailable.is_retryable());
        assert!(!ErrorClass::Authentication.is_retryable());
        assert!(!ErrorClass::NotFound.is_retryable());
    }

    #[test]
    fn test_http_status_classification() {
        assert_eq!(ErrorClass::from_http_status(429), ErrorClass::RateLimit);
        assert_eq!(
            ErrorClass::from_http_status(503),
            ErrorClass::ServiceUnavailable
        );
        assert_eq!(
            ErrorClass::from_http_status(401),
            ErrorClass::Authentication
        );
        assert_eq!(ErrorClass::from_http_status(404), ErrorClass::NotFound);
    }

    #[test]
    fn test_error_message_classification() {
        assert_eq!(
            ErrorClass::from_error_message("Connection timed out"),
            ErrorClass::Network
        );
        assert_eq!(
            ErrorClass::from_error_message("Rate limit exceeded"),
            ErrorClass::RateLimit
        );
    }

    #[test]
    fn test_step_error_retryable() {
        assert!(StepError::retryable("test").is_retryable());
        assert!(StepError::timeout(1000).is_retryable());
        assert!(!StepError::non_retryable("test").is_retryable());
        assert!(!StepError::validation("test").is_retryable());
    }

    #[test]
    fn test_step_error_creation() {
        let error = StepError::retryable("network error");
        assert!(error.is_retryable());
        assert!(error.to_string().contains("network error"));

        let error = StepError::non_retryable("validation failed");
        assert!(!error.is_retryable());
        assert!(error.to_string().contains("validation failed"));

        let error = StepError::timeout(5000);
        assert!(error.is_retryable());
        assert!(error.to_string().contains("5000ms"));

        let error = StepError::validation("invalid input");
        assert!(!error.is_retryable());
        assert!(error.to_string().contains("invalid input"));
    }

    #[test]
    fn test_on_error_behavior_default() {
        assert_eq!(OnErrorBehavior::default(), OnErrorBehavior::Stop);
    }

    #[test]
    fn test_error_class_from_http_status_ranges() {
        // 4xx errors
        assert_eq!(ErrorClass::from_http_status(400), ErrorClass::Validation);
        assert_eq!(
            ErrorClass::from_http_status(401),
            ErrorClass::Authentication
        );
        assert_eq!(ErrorClass::from_http_status(403), ErrorClass::Authorization);
        assert_eq!(ErrorClass::from_http_status(404), ErrorClass::NotFound);
        assert_eq!(ErrorClass::from_http_status(422), ErrorClass::Validation);
        assert_eq!(ErrorClass::from_http_status(429), ErrorClass::RateLimit);

        // 5xx errors
        assert_eq!(ErrorClass::from_http_status(500), ErrorClass::Internal);
        assert_eq!(ErrorClass::from_http_status(502), ErrorClass::Internal);
        assert_eq!(
            ErrorClass::from_http_status(503),
            ErrorClass::ServiceUnavailable
        );
        assert_eq!(ErrorClass::from_http_status(504), ErrorClass::Internal);
        assert_eq!(ErrorClass::from_http_status(599), ErrorClass::Internal);
    }

    #[test]
    fn test_error_class_from_message_variations() {
        // Network variations
        assert_eq!(
            ErrorClass::from_error_message("Connection timeout"),
            ErrorClass::Network
        );
        assert_eq!(
            ErrorClass::from_error_message("Request timed out"),
            ErrorClass::Network
        );
        assert_eq!(
            ErrorClass::from_error_message("Network error occurred"),
            ErrorClass::Network
        );
        assert_eq!(
            ErrorClass::from_error_message("Connection refused"),
            ErrorClass::Network
        );

        // Rate limit variations
        assert_eq!(
            ErrorClass::from_error_message("Rate limit exceeded"),
            ErrorClass::RateLimit
        );
        assert_eq!(
            ErrorClass::from_error_message("Too many requests"),
            ErrorClass::RateLimit
        );

        // Auth variations
        assert_eq!(
            ErrorClass::from_error_message("Unauthorized access"),
            ErrorClass::Authentication
        );
        assert_eq!(
            ErrorClass::from_error_message("Authentication failed"),
            ErrorClass::Authentication
        );
        assert_eq!(
            ErrorClass::from_error_message("Forbidden resource"),
            ErrorClass::Authorization
        );
        assert_eq!(
            ErrorClass::from_error_message("Permission denied"),
            ErrorClass::Authorization
        );

        // Validation variations
        assert_eq!(
            ErrorClass::from_error_message("Invalid input"),
            ErrorClass::Validation
        );
        assert_eq!(
            ErrorClass::from_error_message("Validation failed"),
            ErrorClass::Validation
        );

        // Not found
        assert_eq!(
            ErrorClass::from_error_message("Resource not found"),
            ErrorClass::NotFound
        );

        // Service availability
        assert_eq!(
            ErrorClass::from_error_message("Service unavailable"),
            ErrorClass::ServiceUnavailable
        );

        // Unknown
        assert_eq!(
            ErrorClass::from_error_message("Something went wrong"),
            ErrorClass::Unknown
        );
    }
}
