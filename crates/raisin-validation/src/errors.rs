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

//! Error types and codes for validation.
//!
//! This module defines structured validation errors with consistent error codes
//! and severity levels that can be used across CLI and server implementations.

/// Standard error codes for validation failures.
///
/// These codes provide machine-readable identifiers for different validation
/// error types, allowing clients to handle specific errors programmatically.
pub mod codes {
    /// A required field is missing from the data
    pub const MISSING_REQUIRED_FIELD: &str = "MISSING_REQUIRED_FIELD";

    /// A required field on an element type is missing
    pub const MISSING_REQUIRED_ELEMENT_FIELD: &str = "MISSING_REQUIRED_ELEMENT_FIELD";

    /// A required field on an archetype is missing
    pub const MISSING_REQUIRED_ARCHETYPE_FIELD: &str = "MISSING_REQUIRED_ARCHETYPE_FIELD";

    /// Referenced element type does not exist
    pub const UNKNOWN_ELEMENT_TYPE: &str = "UNKNOWN_ELEMENT_TYPE";

    /// Circular inheritance detected in type hierarchy
    pub const CIRCULAR_INHERITANCE: &str = "CIRCULAR_INHERITANCE";

    /// Strict mode violation - unexpected field encountered
    pub const STRICT_MODE_VIOLATION: &str = "STRICT_MODE_VIOLATION";

    /// Maximum inheritance depth exceeded
    pub const MAX_INHERITANCE_DEPTH: &str = "MAX_INHERITANCE_DEPTH";

    /// Invalid field value type or format
    pub const INVALID_FIELD_VALUE: &str = "INVALID_FIELD_VALUE";
}

/// Severity level for validation errors.
///
/// Allows distinguishing between hard errors that prevent processing
/// and warnings that may be informational.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Critical error that prevents further processing
    Error,
    /// Non-critical issue that should be reviewed
    Warning,
}

/// A structured validation error with code, path, and message.
///
/// This type provides rich context about validation failures, including
/// the location in the data structure where the error occurred.
#[derive(Debug, Clone)]
pub struct ValidationError {
    /// Machine-readable error code from the `codes` module
    pub code: &'static str,

    /// Dot-separated path to the field or location where error occurred
    /// Example: "element.fields.title"
    pub path: String,

    /// Human-readable error message
    pub message: String,

    /// Severity level of this error
    pub severity: Severity,
}

impl ValidationError {
    /// Create a new validation error for a missing required field.
    ///
    /// # Arguments
    ///
    /// * `path` - The location path where the field should exist
    /// * `field_name` - The name of the missing field
    pub fn missing_required(path: &str, field_name: &str) -> Self {
        Self {
            code: codes::MISSING_REQUIRED_FIELD,
            path: path.to_string(),
            message: format!("Missing required field: {}", field_name),
            severity: Severity::Error,
        }
    }

    /// Create a validation error for an unknown element type reference.
    ///
    /// # Arguments
    ///
    /// * `path` - The location path of the reference
    /// * `element_type` - The name of the unknown element type
    pub fn unknown_element_type(path: &str, element_type: &str) -> Self {
        Self {
            code: codes::UNKNOWN_ELEMENT_TYPE,
            path: path.to_string(),
            message: format!("Unknown element type: {}", element_type),
            severity: Severity::Error,
        }
    }

    /// Create a validation error for circular inheritance.
    ///
    /// # Arguments
    ///
    /// * `type_name` - The name of the type with circular inheritance
    /// * `chain` - The inheritance chain showing the cycle
    pub fn circular_inheritance(type_name: &str, chain: &[String]) -> Self {
        Self {
            code: codes::CIRCULAR_INHERITANCE,
            path: type_name.to_string(),
            message: format!("Circular inheritance detected: {}", chain.join(" -> ")),
            severity: Severity::Error,
        }
    }

    /// Create a validation error for exceeding maximum inheritance depth.
    ///
    /// # Arguments
    ///
    /// * `type_name` - The name of the type
    /// * `max_depth` - The maximum allowed depth
    pub fn max_inheritance_depth(type_name: &str, max_depth: usize) -> Self {
        Self {
            code: codes::MAX_INHERITANCE_DEPTH,
            path: type_name.to_string(),
            message: format!("Inheritance depth exceeds maximum of {}", max_depth),
            severity: Severity::Error,
        }
    }

    /// Create a validation error for strict mode violations.
    ///
    /// # Arguments
    ///
    /// * `path` - The location path
    /// * `field_name` - The unexpected field name
    pub fn strict_mode_violation(path: &str, field_name: &str) -> Self {
        Self {
            code: codes::STRICT_MODE_VIOLATION,
            path: path.to_string(),
            message: format!("Unexpected field in strict mode: {}", field_name),
            severity: Severity::Warning,
        }
    }

    /// Create a validation error for invalid field values.
    ///
    /// # Arguments
    ///
    /// * `path` - The location path
    /// * `field_name` - The field name
    /// * `reason` - Description of why the value is invalid
    pub fn invalid_field_value(path: &str, field_name: &str, reason: &str) -> Self {
        Self {
            code: codes::INVALID_FIELD_VALUE,
            path: path.to_string(),
            message: format!("Invalid value for field '{}': {}", field_name, reason),
            severity: Severity::Error,
        }
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {} - {}", self.code, self.path, self.message)
    }
}

impl std::error::Error for ValidationError {}

/// Convert ValidationError to raisin_error::Error.
///
/// This allows ValidationError to be used with the `?` operator in functions
/// that return `raisin_error::Result`.
impl From<ValidationError> for raisin_error::Error {
    fn from(err: ValidationError) -> Self {
        raisin_error::Error::Validation(err.to_string())
    }
}
