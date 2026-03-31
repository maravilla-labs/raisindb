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

//! Helper functions for working with field schemas.
//!
//! This module provides utilities for extracting information from `FieldSchema`
//! variants without needing to match on each variant individually.

use raisin_models::nodes::types::element::field_types::FieldSchema;

/// Check if a field is marked as required.
///
/// A field is considered required if its `base.required` field is `Some(true)`.
/// If `required` is `None` or `Some(false)`, the field is not required.
///
/// # Arguments
///
/// * `field` - The field schema to check
///
/// # Returns
///
/// `true` if the field is required, `false` otherwise
///
/// # Examples
///
/// ```rust,ignore
/// use raisin_validation::is_required;
///
/// if is_required(&field) {
///     println!("Field {} is required", field_name(&field));
/// }
/// ```
pub fn is_required(field: &FieldSchema) -> bool {
    match field {
        FieldSchema::TextField { base, .. }
        | FieldSchema::RichTextField { base, .. }
        | FieldSchema::NumberField { base, .. }
        | FieldSchema::DateField { base, .. }
        | FieldSchema::LocationField { base, .. }
        | FieldSchema::BooleanField { base, .. }
        | FieldSchema::MediaField { base, .. }
        | FieldSchema::ReferenceField { base, .. }
        | FieldSchema::TagField { base, .. }
        | FieldSchema::OptionsField { base, .. }
        | FieldSchema::JsonObjectField { base }
        | FieldSchema::CompositeField { base, .. }
        | FieldSchema::SectionField { base, .. }
        | FieldSchema::ElementField { base, .. }
        | FieldSchema::ListingField { base, .. } => base.required.unwrap_or(false),
    }
}

/// Get the name of a field from any `FieldSchema` variant.
///
/// All field variants contain a `base` field with a `name` property.
/// This function extracts that name as a string slice.
///
/// # Arguments
///
/// * `field` - The field schema
///
/// # Returns
///
/// A string slice containing the field name
///
/// # Examples
///
/// ```rust,ignore
/// use raisin_validation::field_name;
///
/// let name = field_name(&field);
/// println!("Processing field: {}", name);
/// ```
pub fn field_name(field: &FieldSchema) -> &str {
    match field {
        FieldSchema::TextField { base, .. }
        | FieldSchema::RichTextField { base, .. }
        | FieldSchema::NumberField { base, .. }
        | FieldSchema::DateField { base, .. }
        | FieldSchema::LocationField { base, .. }
        | FieldSchema::BooleanField { base, .. }
        | FieldSchema::MediaField { base, .. }
        | FieldSchema::ReferenceField { base, .. }
        | FieldSchema::TagField { base, .. }
        | FieldSchema::OptionsField { base, .. }
        | FieldSchema::JsonObjectField { base }
        | FieldSchema::CompositeField { base, .. }
        | FieldSchema::SectionField { base, .. }
        | FieldSchema::ElementField { base, .. }
        | FieldSchema::ListingField { base, .. } => &base.name,
    }
}

/// Check if a field allows multiple values (is repeatable).
///
/// A field is considered multiple if its `base.multiple` field is `Some(true)`.
/// If `multiple` is `None` or `Some(false)`, the field is single-valued.
///
/// # Arguments
///
/// * `field` - The field schema to check
///
/// # Returns
///
/// `true` if the field allows multiple values, `false` otherwise
///
/// # Examples
///
/// ```rust,ignore
/// use raisin_validation::is_multiple;
///
/// if is_multiple(&field) {
///     println!("Field {} accepts multiple values", field_name(&field));
/// }
/// ```
pub fn is_multiple(field: &FieldSchema) -> bool {
    match field {
        FieldSchema::TextField { base, .. }
        | FieldSchema::RichTextField { base, .. }
        | FieldSchema::NumberField { base, .. }
        | FieldSchema::DateField { base, .. }
        | FieldSchema::LocationField { base, .. }
        | FieldSchema::BooleanField { base, .. }
        | FieldSchema::MediaField { base, .. }
        | FieldSchema::ReferenceField { base, .. }
        | FieldSchema::TagField { base, .. }
        | FieldSchema::OptionsField { base, .. }
        | FieldSchema::JsonObjectField { base }
        | FieldSchema::CompositeField { base, .. }
        | FieldSchema::SectionField { base, .. }
        | FieldSchema::ElementField { base, .. }
        | FieldSchema::ListingField { base, .. } => base.multiple.unwrap_or(false),
    }
}

/// Check if a field is a `SectionField`.
///
/// Section fields are special fields that group other content and may have
/// special rendering or validation requirements.
///
/// # Arguments
///
/// * `field` - The field schema to check
///
/// # Returns
///
/// `true` if the field is a `SectionField`, `false` otherwise
///
/// # Examples
///
/// ```rust,ignore
/// use raisin_validation::is_section_field;
///
/// if is_section_field(&field) {
///     println!("Field {} is a section", field_name(&field));
/// }
/// ```
pub fn is_section_field(field: &FieldSchema) -> bool {
    matches!(field, FieldSchema::SectionField { .. })
}

/// Check if a field is an `ElementField`.
///
/// Element fields reference other element types and may require special
/// validation and resolution logic.
///
/// # Arguments
///
/// * `field` - The field schema to check
///
/// # Returns
///
/// `true` if the field is an `ElementField`, `false` otherwise
///
/// # Examples
///
/// ```rust,ignore
/// use raisin_validation::is_element_field;
///
/// if is_element_field(&field) {
///     println!("Field {} references an element type", field_name(&field));
/// }
/// ```
pub fn is_element_field(field: &FieldSchema) -> bool {
    matches!(field, FieldSchema::ElementField { .. })
}

/// Get the element type name from an `ElementField`.
///
/// If the field is an `ElementField`, returns `Some(&str)` with the element type name.
/// For all other field types, returns `None`.
///
/// # Arguments
///
/// * `field` - The field schema
///
/// # Returns
///
/// `Some(&str)` if the field is an `ElementField`, `None` otherwise
///
/// # Examples
///
/// ```rust,ignore
/// use raisin_validation::element_type_name;
///
/// if let Some(type_name) = element_type_name(&field) {
///     println!("Field references element type: {}", type_name);
/// }
/// ```
pub fn element_type_name(field: &FieldSchema) -> Option<&str> {
    match field {
        FieldSchema::ElementField { element_type, .. } => Some(element_type),
        _ => None,
    }
}

/// Check if a field is hidden on publish.
///
/// A field is considered hidden if its `base.is_hidden` field is `Some(true)`.
/// Hidden fields may be used internally but not displayed or exported.
///
/// # Arguments
///
/// * `field` - The field schema to check
///
/// # Returns
///
/// `true` if the field is hidden, `false` otherwise
pub fn is_hidden(field: &FieldSchema) -> bool {
    match field {
        FieldSchema::TextField { base, .. }
        | FieldSchema::RichTextField { base, .. }
        | FieldSchema::NumberField { base, .. }
        | FieldSchema::DateField { base, .. }
        | FieldSchema::LocationField { base, .. }
        | FieldSchema::BooleanField { base, .. }
        | FieldSchema::MediaField { base, .. }
        | FieldSchema::ReferenceField { base, .. }
        | FieldSchema::TagField { base, .. }
        | FieldSchema::OptionsField { base, .. }
        | FieldSchema::JsonObjectField { base }
        | FieldSchema::CompositeField { base, .. }
        | FieldSchema::SectionField { base, .. }
        | FieldSchema::ElementField { base, .. }
        | FieldSchema::ListingField { base, .. } => base.is_hidden.unwrap_or(false),
    }
}

/// Check if a field is marked as a design value field.
///
/// Design value fields may be used for styling or layout configuration
/// rather than content data.
///
/// # Arguments
///
/// * `field` - The field schema to check
///
/// # Returns
///
/// `true` if the field is a design value field, `false` otherwise
pub fn is_design_value(field: &FieldSchema) -> bool {
    match field {
        FieldSchema::TextField { base, .. }
        | FieldSchema::RichTextField { base, .. }
        | FieldSchema::NumberField { base, .. }
        | FieldSchema::DateField { base, .. }
        | FieldSchema::LocationField { base, .. }
        | FieldSchema::BooleanField { base, .. }
        | FieldSchema::MediaField { base, .. }
        | FieldSchema::ReferenceField { base, .. }
        | FieldSchema::TagField { base, .. }
        | FieldSchema::OptionsField { base, .. }
        | FieldSchema::JsonObjectField { base }
        | FieldSchema::CompositeField { base, .. }
        | FieldSchema::SectionField { base, .. }
        | FieldSchema::ElementField { base, .. }
        | FieldSchema::ListingField { base, .. } => base.design_value.unwrap_or(false),
    }
}

/// Check if a field is translatable.
///
/// Translatable fields support multiple language versions of their content.
///
/// # Arguments
///
/// * `field` - The field schema to check
///
/// # Returns
///
/// `true` if the field is translatable, `false` otherwise
pub fn is_translatable(field: &FieldSchema) -> bool {
    match field {
        FieldSchema::TextField { base, .. }
        | FieldSchema::RichTextField { base, .. }
        | FieldSchema::NumberField { base, .. }
        | FieldSchema::DateField { base, .. }
        | FieldSchema::LocationField { base, .. }
        | FieldSchema::BooleanField { base, .. }
        | FieldSchema::MediaField { base, .. }
        | FieldSchema::ReferenceField { base, .. }
        | FieldSchema::TagField { base, .. }
        | FieldSchema::OptionsField { base, .. }
        | FieldSchema::JsonObjectField { base }
        | FieldSchema::CompositeField { base, .. }
        | FieldSchema::SectionField { base, .. }
        | FieldSchema::ElementField { base, .. }
        | FieldSchema::ListingField { base, .. } => base.translatable.unwrap_or(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::nodes::types::element::fields::base_field::FieldTypeSchema;

    fn create_text_field(
        name: &str,
        required: Option<bool>,
        multiple: Option<bool>,
    ) -> FieldSchema {
        FieldSchema::TextField {
            base: FieldTypeSchema {
                name: name.to_string(),
                required,
                multiple,
                ..Default::default()
            },
            config: None,
        }
    }

    #[test]
    fn test_is_required() {
        let required_field = create_text_field("test", Some(true), None);
        assert!(is_required(&required_field));

        let optional_field = create_text_field("test", Some(false), None);
        assert!(!is_required(&optional_field));

        let default_field = create_text_field("test", None, None);
        assert!(!is_required(&default_field));
    }

    #[test]
    fn test_field_name() {
        let field = create_text_field("my_field", None, None);
        assert_eq!(field_name(&field), "my_field");
    }

    #[test]
    fn test_is_multiple() {
        let multiple_field = create_text_field("test", None, Some(true));
        assert!(is_multiple(&multiple_field));

        let single_field = create_text_field("test", None, Some(false));
        assert!(!is_multiple(&single_field));

        let default_field = create_text_field("test", None, None);
        assert!(!is_multiple(&default_field));
    }

    #[test]
    fn test_is_section_field() {
        let text_field = create_text_field("test", None, None);
        assert!(!is_section_field(&text_field));

        let section_field = FieldSchema::SectionField {
            base: FieldTypeSchema {
                name: "section".to_string(),
                ..Default::default()
            },
            allowed_element_types: None,
            render_as: None,
        };
        assert!(is_section_field(&section_field));
    }

    #[test]
    fn test_is_element_field() {
        let text_field = create_text_field("test", None, None);
        assert!(!is_element_field(&text_field));

        let element_field = FieldSchema::ElementField {
            base: FieldTypeSchema {
                name: "element".to_string(),
                ..Default::default()
            },
            element_type: "Article".to_string(),
        };
        assert!(is_element_field(&element_field));
    }

    #[test]
    fn test_element_type_name() {
        let text_field = create_text_field("test", None, None);
        assert_eq!(element_type_name(&text_field), None);

        let element_field = FieldSchema::ElementField {
            base: FieldTypeSchema {
                name: "element".to_string(),
                ..Default::default()
            },
            element_type: "Article".to_string(),
        };
        assert_eq!(element_type_name(&element_field), Some("Article"));
    }
}
