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

//! Schema validation for field values.
//!
//! This module provides validation functions for checking field values
//! against their schema definitions, including required field checks.

use crate::errors::ValidationError;
use crate::field_helpers::{field_name, is_required};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::types::element::field_types::FieldSchema;
use std::collections::HashMap;

/// Validate field values against their schema definitions.
///
/// This function checks that all required fields are present and have non-null values.
/// It calls the provided error handler for each validation error found.
///
/// # Arguments
///
/// * `fields` - The field schema definitions to validate against
/// * `values` - The actual field values provided
/// * `path` - The path prefix for error reporting (e.g., "element.fields")
/// * `on_error` - Callback function invoked for each validation error
///
/// # Type Parameters
///
/// * `F` - The error handler function type
///
/// # Examples
///
/// ```rust,ignore
/// use raisin_validation::validate_fields;
/// use std::collections::HashMap;
///
/// let mut errors = Vec::new();
/// validate_fields(&fields, &values, "element.fields", |err| {
///     errors.push(err);
/// });
///
/// if !errors.is_empty() {
///     println!("Found {} validation errors", errors.len());
/// }
/// ```
pub fn validate_fields<F>(
    fields: &[FieldSchema],
    values: &HashMap<String, PropertyValue>,
    path: &str,
    mut on_error: F,
) where
    F: FnMut(ValidationError),
{
    for field in fields {
        let name = field_name(field);
        let value = values.get(name);

        // Build the full path for this field
        let field_path = if path.is_empty() {
            name.to_string()
        } else {
            format!("{}.{}", path, name)
        };

        // Check required fields
        if is_required(field) {
            match value {
                None => {
                    // Field is missing entirely
                    on_error(ValidationError::missing_required(&field_path, name));
                }
                Some(PropertyValue::Null) => {
                    // Field is present but null
                    on_error(ValidationError::missing_required(&field_path, name));
                }
                Some(_) => {
                    // Field has a value, validation passes
                }
            }
        }

        // Additional validation logic can be added here:
        // - Type checking (ensure value matches field type)
        // - Format validation (regex, min/max, etc.)
        // - Reference validation (ensure referenced entities exist)
        // - Nested validation for composite fields
    }
}

/// Validate field values in strict mode.
///
/// In addition to required field validation, this checks that no extra fields
/// are present that aren't defined in the schema. Extra fields generate warnings.
///
/// # Arguments
///
/// * `fields` - The field schema definitions to validate against
/// * `values` - The actual field values provided
/// * `path` - The path prefix for error reporting
/// * `on_error` - Callback function invoked for each validation error
///
/// # Examples
///
/// ```rust,ignore
/// use raisin_validation::validate_fields_strict;
///
/// let mut errors = Vec::new();
/// validate_fields_strict(&fields, &values, "element.fields", |err| {
///     errors.push(err);
/// });
/// ```
pub fn validate_fields_strict<F>(
    fields: &[FieldSchema],
    values: &HashMap<String, PropertyValue>,
    path: &str,
    mut on_error: F,
) where
    F: FnMut(ValidationError),
{
    // First, perform standard validation
    validate_fields(fields, values, path, &mut on_error);

    // Build set of valid field names
    let valid_names: std::collections::HashSet<&str> = fields.iter().map(field_name).collect();

    // Check for unexpected fields
    for value_name in values.keys() {
        if !valid_names.contains(value_name.as_str()) {
            let field_path = if path.is_empty() {
                value_name.clone()
            } else {
                format!("{}.{}", path, value_name)
            };
            on_error(ValidationError::strict_mode_violation(
                &field_path,
                value_name,
            ));
        }
    }
}

/// Collect all validation errors into a vector.
///
/// This is a convenience wrapper around `validate_fields` that collects
/// errors into a vector rather than requiring a closure.
///
/// # Arguments
///
/// * `fields` - The field schema definitions to validate against
/// * `values` - The actual field values provided
/// * `path` - The path prefix for error reporting
///
/// # Returns
///
/// A vector of all validation errors found. Empty if validation passes.
///
/// # Examples
///
/// ```rust,ignore
/// use raisin_validation::collect_validation_errors;
///
/// let errors = collect_validation_errors(&fields, &values, "element.fields");
/// if !errors.is_empty() {
///     eprintln!("Validation failed with {} errors", errors.len());
///     for error in errors {
///         eprintln!("  {}", error);
///     }
/// }
/// ```
pub fn collect_validation_errors(
    fields: &[FieldSchema],
    values: &HashMap<String, PropertyValue>,
    path: &str,
) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    validate_fields(fields, values, path, |err| {
        errors.push(err);
    });
    errors
}

/// Collect all validation errors in strict mode into a vector.
///
/// This is a convenience wrapper around `validate_fields_strict` that collects
/// errors into a vector.
///
/// # Arguments
///
/// * `fields` - The field schema definitions to validate against
/// * `values` - The actual field values provided
/// * `path` - The path prefix for error reporting
///
/// # Returns
///
/// A vector of all validation errors found. Empty if validation passes.
pub fn collect_validation_errors_strict(
    fields: &[FieldSchema],
    values: &HashMap<String, PropertyValue>,
    path: &str,
) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    validate_fields_strict(fields, values, path, |err| {
        errors.push(err);
    });
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::nodes::types::element::fields::base_field::FieldTypeSchema;

    fn create_required_field(name: &str) -> FieldSchema {
        FieldSchema::TextField {
            base: FieldTypeSchema {
                name: name.to_string(),
                required: Some(true),
                ..Default::default()
            },
            config: None,
        }
    }

    fn create_optional_field(name: &str) -> FieldSchema {
        FieldSchema::TextField {
            base: FieldTypeSchema {
                name: name.to_string(),
                required: Some(false),
                ..Default::default()
            },
            config: None,
        }
    }

    #[test]
    fn test_validate_required_field_missing() {
        let fields = vec![create_required_field("title")];
        let values = HashMap::new();

        let errors = collect_validation_errors(&fields, &values, "test");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, crate::errors::codes::MISSING_REQUIRED_FIELD);
    }

    #[test]
    fn test_validate_required_field_null() {
        let fields = vec![create_required_field("title")];
        let mut values = HashMap::new();
        values.insert("title".to_string(), PropertyValue::Null);

        let errors = collect_validation_errors(&fields, &values, "test");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, crate::errors::codes::MISSING_REQUIRED_FIELD);
    }

    #[test]
    fn test_validate_required_field_present() {
        let fields = vec![create_required_field("title")];
        let mut values = HashMap::new();
        values.insert(
            "title".to_string(),
            PropertyValue::String("Test Title".to_string()),
        );

        let errors = collect_validation_errors(&fields, &values, "test");
        assert_eq!(errors.len(), 0);
    }

    #[test]
    fn test_validate_optional_field_missing() {
        let fields = vec![create_optional_field("description")];
        let values = HashMap::new();

        let errors = collect_validation_errors(&fields, &values, "test");
        assert_eq!(errors.len(), 0);
    }

    #[test]
    fn test_validate_strict_mode_extra_field() {
        let fields = vec![create_required_field("title")];
        let mut values = HashMap::new();
        values.insert(
            "title".to_string(),
            PropertyValue::String("Test".to_string()),
        );
        values.insert(
            "extra_field".to_string(),
            PropertyValue::String("Extra".to_string()),
        );

        let errors = collect_validation_errors_strict(&fields, &values, "test");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, crate::errors::codes::STRICT_MODE_VIOLATION);
    }

    #[test]
    fn test_validate_strict_mode_no_extra_fields() {
        let fields = vec![create_required_field("title")];
        let mut values = HashMap::new();
        values.insert(
            "title".to_string(),
            PropertyValue::String("Test".to_string()),
        );

        let errors = collect_validation_errors_strict(&fields, &values, "test");
        assert_eq!(errors.len(), 0);
    }

    #[test]
    fn test_error_path_construction() {
        let fields = vec![create_required_field("title")];
        let values = HashMap::new();

        let errors = collect_validation_errors(&fields, &values, "element.fields");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].path, "element.fields.title");
    }
}
