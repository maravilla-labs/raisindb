//! Output validation for AI responses using JSON schema.
//!
//! This module provides runtime validation of AI-generated JSON outputs against
//! JSON schemas. It's particularly useful for validating function outputs to
//! ensure they conform to expected structures before processing.
//!
//! # Examples
//!
//! ```rust
//! use raisin_ai::validation::validate_output;
//! use serde_json::json;
//!
//! let schema = json!({
//!     "type": "object",
//!     "properties": {
//!         "name": { "type": "string" },
//!         "age": { "type": "number" }
//!     },
//!     "required": ["name", "age"]
//! });
//!
//! let valid_output = json!({
//!     "name": "Alice",
//!     "age": 30
//! });
//!
//! let invalid_output = json!({
//!     "name": "Bob"
//!     // Missing required "age" field
//! });
//!
//! assert!(validate_output(&schema, &valid_output).is_ok());
//! assert!(validate_output(&schema, &invalid_output).is_err());
//! ```

use serde_json::Value;
use thiserror::Error;

/// Errors that can occur during JSON schema validation.
#[derive(Debug, Error)]
pub enum ValidationError {
    /// The provided JSON schema is invalid and cannot be compiled.
    ///
    /// This typically indicates a malformed schema definition, such as
    /// invalid syntax or unsupported schema features.
    #[error("Schema compilation failed: {0}")]
    SchemaCompilationError(String),

    /// The output failed validation against the schema.
    ///
    /// Contains a list of validation error messages describing what failed.
    /// These messages can be used to provide feedback for model retries.
    #[error("Validation failed: {errors:?}")]
    ValidationFailed {
        /// Detailed list of validation errors
        errors: Vec<String>,
    },
}

/// Validates a JSON value against a JSON schema.
///
/// This function compiles the provided JSON schema and validates the output
/// against it. If validation succeeds, returns `Ok(())`. If validation fails,
/// returns a `ValidationError` with detailed error messages.
///
/// # Arguments
///
/// * `schema` - A JSON schema definition (as a `serde_json::Value`)
/// * `output` - The JSON output to validate
///
/// # Returns
///
/// `Ok(())` if validation succeeds, or a `ValidationError` describing what failed.
///
/// # Errors
///
/// - `ValidationError::SchemaCompilationError` - If the schema cannot be compiled
/// - `ValidationError::ValidationFailed` - If the output doesn't match the schema
///
/// # Examples
///
/// ```rust
/// use raisin_ai::validation::validate_output;
/// use serde_json::json;
///
/// let schema = json!({
///     "type": "object",
///     "properties": {
///         "status": { "type": "string" }
///     }
/// });
///
/// let output = json!({ "status": "success" });
/// assert!(validate_output(&schema, &output).is_ok());
/// ```
pub fn validate_output(schema: &Value, output: &Value) -> Result<(), ValidationError> {
    // Compile the schema
    let compiled = jsonschema::validator_for(schema).map_err(|e| {
        ValidationError::SchemaCompilationError(format!("Failed to compile schema: {}", e))
    })?;

    // Validate the output
    if let Err(error) = compiled.validate(output) {
        // Collect all validation errors into a Vec<String>
        let error_messages: Vec<String> = vec![format!("{}", error)];

        Err(ValidationError::ValidationFailed {
            errors: error_messages,
        })
    } else {
        Ok(())
    }
}

/// Validates output and returns detailed error messages suitable for model retry.
///
/// This function is similar to `validate_output` but formats error messages in a way
/// that's more suitable for providing feedback to AI models for retry attempts.
/// The error messages are designed to be clear and actionable.
///
/// # Arguments
///
/// * `schema` - A JSON schema definition (as a `serde_json::Value`)
/// * `output` - The JSON output to validate
///
/// # Returns
///
/// `Ok(())` if validation succeeds, or a `ValidationError` with detailed,
/// AI-friendly error messages.
///
/// # Errors
///
/// - `ValidationError::SchemaCompilationError` - If the schema cannot be compiled
/// - `ValidationError::ValidationFailed` - If the output doesn't match the schema,
///   with detailed messages suitable for AI model feedback
///
/// # Examples
///
/// ```rust
/// use raisin_ai::validation::validate_with_details;
/// use serde_json::json;
///
/// let schema = json!({
///     "type": "object",
///     "properties": {
///         "count": { "type": "number", "minimum": 0 }
///     },
///     "required": ["count"]
/// });
///
/// let invalid_output = json!({ "count": "not a number" });
///
/// match validate_with_details(&schema, &invalid_output) {
///     Err(e) => println!("Validation errors for retry: {}", e),
///     Ok(_) => println!("Valid!"),
/// }
/// ```
pub fn validate_with_details(schema: &Value, output: &Value) -> Result<(), ValidationError> {
    // Compile the schema
    let compiled = jsonschema::validator_for(schema).map_err(|e| {
        ValidationError::SchemaCompilationError(format!(
            "The provided schema is invalid: {}. Please check the schema definition.",
            e
        ))
    })?;

    // Validate the output
    if let Err(error) = compiled.validate(output) {
        // Collect detailed, AI-friendly error messages
        let path_str = error.instance_path().to_string();
        let path = if path_str.is_empty() {
            "root".to_string()
        } else {
            path_str
        };

        let error_messages = vec![format!("Validation error: {} (location: {})", error, path)];

        Err(ValidationError::ValidationFailed {
            errors: error_messages,
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_valid_simple_object() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "number" }
            },
            "required": ["name", "age"]
        });

        let output = json!({
            "name": "Alice",
            "age": 30
        });

        assert!(validate_output(&schema, &output).is_ok());
    }

    #[test]
    fn test_missing_required_field() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "number" }
            },
            "required": ["name", "age"]
        });

        let output = json!({
            "name": "Bob"
        });

        let result = validate_output(&schema, &output);
        assert!(result.is_err());

        match result {
            Err(ValidationError::ValidationFailed { errors }) => {
                assert!(!errors.is_empty());
            }
            _ => panic!("Expected ValidationFailed error"),
        }
    }

    #[test]
    fn test_wrong_type() {
        let schema = json!({
            "type": "object",
            "properties": {
                "count": { "type": "number" }
            }
        });

        let output = json!({
            "count": "not a number"
        });

        let result = validate_output(&schema, &output);
        assert!(result.is_err());
    }

    #[test]
    fn test_nested_object_validation() {
        let schema = json!({
            "type": "object",
            "properties": {
                "user": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "email": { "type": "string", "format": "email" }
                    },
                    "required": ["name", "email"]
                }
            },
            "required": ["user"]
        });

        let valid_output = json!({
            "user": {
                "name": "Charlie",
                "email": "charlie@example.com"
            }
        });

        assert!(validate_output(&schema, &valid_output).is_ok());

        let invalid_output = json!({
            "user": {
                "name": "Charlie"
            }
        });

        assert!(validate_output(&schema, &invalid_output).is_err());
    }

    #[test]
    fn test_array_validation() {
        let schema = json!({
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "id": { "type": "number" },
                    "name": { "type": "string" }
                },
                "required": ["id", "name"]
            }
        });

        let valid_output = json!([
            { "id": 1, "name": "Item 1" },
            { "id": 2, "name": "Item 2" }
        ]);

        assert!(validate_output(&schema, &valid_output).is_ok());

        let invalid_output = json!([
            { "id": 1, "name": "Item 1" },
            { "id": "not a number", "name": "Item 2" }
        ]);

        assert!(validate_output(&schema, &invalid_output).is_err());
    }

    #[test]
    fn test_invalid_schema() {
        // Invalid schema with malformed type
        let schema = json!({
            "type": "invalid_type"
        });

        let output = json!({ "test": "value" });

        let result = validate_output(&schema, &output);
        assert!(result.is_err());

        match result {
            Err(ValidationError::SchemaCompilationError(_)) => {}
            _ => panic!("Expected SchemaCompilationError"),
        }
    }

    #[test]
    fn test_validate_with_details() {
        let schema = json!({
            "type": "object",
            "properties": {
                "status": { "type": "string" },
                "count": { "type": "number", "minimum": 0 }
            },
            "required": ["status", "count"]
        });

        let invalid_output = json!({
            "status": 123,  // Wrong type
            "count": -5      // Violates minimum
        });

        let result = validate_with_details(&schema, &invalid_output);
        assert!(result.is_err());

        match result {
            Err(ValidationError::ValidationFailed { errors }) => {
                // Should have detailed error messages
                assert!(!errors.is_empty());
                // Error message should contain "Validation error:" prefix
                assert!(errors.iter().any(|e| e.contains("Validation error:")));
            }
            _ => panic!("Expected ValidationFailed error"),
        }
    }

    #[test]
    fn test_additional_properties() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            },
            "additionalProperties": false
        });

        let valid_output = json!({
            "name": "Alice"
        });

        assert!(validate_output(&schema, &valid_output).is_ok());

        let invalid_output = json!({
            "name": "Alice",
            "extra": "not allowed"
        });

        assert!(validate_output(&schema, &invalid_output).is_err());
    }

    #[test]
    fn test_enum_validation() {
        let schema = json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "enum": ["pending", "active", "completed"]
                }
            }
        });

        let valid_output = json!({
            "status": "active"
        });

        assert!(validate_output(&schema, &valid_output).is_ok());

        let invalid_output = json!({
            "status": "invalid_status"
        });

        assert!(validate_output(&schema, &invalid_output).is_err());
    }
}
