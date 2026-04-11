//! Field resolution with inheritance support for archetypes and element types

use crate::errors::{codes, ValidationError, ValidationResult};
use raisin_models::nodes::types::element::field_types::{FieldSchema, FieldSchemaBase};
use raisin_validation::field_helpers::{composite_requires_uuid, is_required as get_field_required};
use serde_yaml::Value;
use std::collections::HashSet;

use super::context::ValidationContext;

/// Resolve an element type with all inherited fields from parent types
pub(crate) fn resolve_element_type_fields(
    element_type_name: &str,
    ctx: &ValidationContext,
    visited: &mut HashSet<String>,
) -> Vec<FieldSchema> {
    // Cycle detection
    if visited.contains(element_type_name) {
        return Vec::new();
    }
    visited.insert(element_type_name.to_string());

    let Some(element_type) = ctx.get_element_type(element_type_name) else {
        return Vec::new();
    };

    let mut resolved_fields = Vec::new();

    // First, get parent fields (if extends is set)
    if let Some(parent_name) = &element_type.extends {
        let parent_fields = resolve_element_type_fields(parent_name, ctx, visited);
        resolved_fields.extend(parent_fields);
    }

    // Then add/override with this type's fields
    for field in &element_type.fields {
        let field_name = field.base_name();
        // Remove existing field with same name (child overrides parent)
        resolved_fields.retain(|f: &FieldSchema| f.base_name() != field_name);
        resolved_fields.push(field.clone());
    }

    resolved_fields
}

/// Resolve an archetype with all inherited fields from parent archetypes
pub(crate) fn resolve_archetype_fields(
    archetype_name: &str,
    ctx: &ValidationContext,
    visited: &mut HashSet<String>,
) -> Vec<FieldSchema> {
    if visited.contains(archetype_name) {
        return Vec::new();
    }
    visited.insert(archetype_name.to_string());

    let Some(archetype) = ctx.get_archetype(archetype_name) else {
        return Vec::new();
    };

    let mut resolved_fields = Vec::new();

    // Get parent fields first
    if let Some(parent_name) = &archetype.extends {
        let parent_fields = resolve_archetype_fields(parent_name, ctx, visited);
        resolved_fields.extend(parent_fields);
    }

    // Add/override with this archetype's fields
    if let Some(fields) = &archetype.fields {
        for field in fields {
            let field_name = field.base_name();
            resolved_fields.retain(|f: &FieldSchema| f.base_name() != field_name);
            resolved_fields.push(field.clone());
        }
    }

    resolved_fields
}

/// Validate element content against resolved element type fields
pub(crate) fn validate_element_content(
    content: &serde_yaml::Mapping,
    element_type_name: &str,
    ctx: &ValidationContext,
    file_path: &str,
    result: &mut ValidationResult,
) {
    // Resolve all fields including inherited ones
    let mut visited = HashSet::new();
    let resolved_fields = resolve_element_type_fields(element_type_name, ctx, &mut visited);

    validate_fields_against_content(&resolved_fields, content, element_type_name, ctx, file_path, result);
}

/// Recursively validate fields against content
pub(crate) fn validate_fields_against_content(
    fields: &[FieldSchema],
    content: &serde_yaml::Mapping,
    context_name: &str,
    ctx: &ValidationContext,
    file_path: &str,
    result: &mut ValidationResult,
) {
    for field in fields {
        let field_name = field.base_name();
        let is_required = get_field_required(field);
        let content_value = content.get(&Value::String(field_name.clone()));

        // Check required fields
        if is_required && content_value.is_none() {
            result.add_error(ValidationError::error(
                file_path,
                context_name,
                codes::MISSING_REQUIRED_ELEMENT_FIELD,
                format!(
                    "Missing required field '{}' for '{}'",
                    field_name, context_name
                ),
            ));
        }

        // Recursively validate nested structures
        if let Some(value) = content_value {
            validate_nested_field(field, value, context_name, ctx, file_path, result);
        }
    }
}

/// Validate nested field content based on field type
fn validate_nested_field(
    field: &FieldSchema,
    value: &Value,
    context_name: &str,
    ctx: &ValidationContext,
    file_path: &str,
    result: &mut ValidationResult,
) {
    match field {
        // CompositeField: validate each array item against nested fields
        FieldSchema::CompositeField { fields, base, .. } => {
            let field_name = &base.name;
            let requires_uuid = composite_requires_uuid(field);

            // CompositeField content can be an array (repeatable) or object (single)
            match value {
                Value::Sequence(items) => {
                    // When multiple + translatable sub-fields, require unique UUIDs
                    if requires_uuid {
                        let mut seen_uuids = HashSet::new();
                        for (i, item) in items.iter().enumerate() {
                            if let Value::Mapping(item_map) = item {
                                let uuid = item_map
                                    .get(&Value::String("uuid".to_string()))
                                    .and_then(|v| v.as_str());
                                match uuid {
                                    None => {
                                        result.add_error(ValidationError::error(
                                            file_path,
                                            &format!("{}[{}]", field_name, i),
                                            codes::COMPOSITE_MISSING_UUID,
                                            format!(
                                                "Item {}[{}] requires a 'uuid' field because the composite has translatable sub-fields",
                                                field_name, i
                                            ),
                                        ));
                                    }
                                    Some(u) if !seen_uuids.insert(u.to_string()) => {
                                        result.add_error(ValidationError::error(
                                            file_path,
                                            &format!("{}[{}].uuid", field_name, i),
                                            codes::COMPOSITE_DUPLICATE_UUID,
                                            format!(
                                                "Duplicate uuid '{}' in composite at {}[{}]",
                                                u, field_name, i
                                            ),
                                        ));
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }

                    // Repeatable: validate each item
                    for (i, item) in items.iter().enumerate() {
                        if let Value::Mapping(item_map) = item {
                            let item_context = format!("{}[{}]", field_name, i);
                            validate_fields_against_content(
                                fields, item_map, &item_context, ctx, file_path, result
                            );
                        }
                    }
                }
                Value::Mapping(item_map) => {
                    // Single item
                    validate_fields_against_content(
                        fields, item_map, field_name, ctx, file_path, result
                    );
                }
                _ => {}
            }
        }

        // SectionField: validate each element against its element type
        FieldSchema::SectionField { base, allowed_element_types, .. } => {
            validate_section_field(&base.name, allowed_element_types, value, ctx, file_path, result);
        }

        // ElementField: validate inline element against referenced type
        FieldSchema::ElementField { element_type, .. } => {
            if let Value::Mapping(elem_map) = value {
                if ctx.is_valid_element_type_ref(element_type) {
                    validate_element_content(elem_map, element_type, ctx, file_path, result);
                }
            }
        }

        // Other field types don't have nested validation
        _ => {}
    }
}

/// Validate a SectionField's elements against their element types
fn validate_section_field(
    field_name: &str,
    allowed_element_types: &Option<Vec<String>>,
    value: &Value,
    ctx: &ValidationContext,
    file_path: &str,
    result: &mut ValidationResult,
) {

    let Value::Sequence(elements) = value else {
        return;
    };

    for (i, element) in elements.iter().enumerate() {
        let Value::Mapping(elem_map) = element else {
            continue;
        };

        // Elements can use $type for inline format
        if let Some(Value::String(elem_type)) = elem_map.get(&Value::String("$type".to_string())) {
            validate_section_element_type(
                elem_type, allowed_element_types, field_name, i,
                elem_map, None, ctx, file_path, result,
            );
        }
        // Or element_type format (with nested content OR flat format)
        else if let Some(Value::String(elem_type)) = elem_map.get(&Value::String("element_type".to_string())) {
            let nested_content = elem_map
                .get(&Value::String("content".to_string()))
                .and_then(|v| v.as_mapping());

            validate_section_element_type(
                elem_type, allowed_element_types, field_name, i,
                elem_map, nested_content, ctx, file_path, result,
            );
        }
    }
}

/// Validate a single element within a SectionField
#[allow(clippy::too_many_arguments)]
fn validate_section_element_type(
    elem_type: &str,
    allowed_element_types: &Option<Vec<String>>,
    field_name: &str,
    index: usize,
    elem_map: &serde_yaml::Mapping,
    nested_content: Option<&serde_yaml::Mapping>,
    ctx: &ValidationContext,
    file_path: &str,
    result: &mut ValidationResult,
) {
    let type_field = if nested_content.is_some() || elem_map.contains_key(&Value::String("element_type".to_string())) {
        "element_type"
    } else {
        "$type"
    };

    // Validate against allowed types if specified
    if let Some(allowed) = allowed_element_types {
        if !allowed.is_empty() && !allowed.contains(&elem_type.to_string()) && !allowed.contains(&"*".to_string()) {
            result.add_warning(ValidationError::warning(
                file_path,
                &format!("{}[{}].{}", field_name, index, type_field),
                "DISALLOWED_ELEMENT_TYPE",
                format!(
                    "Element type '{}' is not in allowed types: {:?}",
                    elem_type, allowed
                ),
            ));
        }
    }

    // Validate element content against its type
    if ctx.is_valid_element_type_ref(elem_type) {
        if let Some(content) = nested_content {
            validate_element_content(content, elem_type, ctx, file_path, result);
        } else {
            validate_element_content(elem_map, elem_type, ctx, file_path, result);
        }
    } else if type_field == "element_type" {
        // Element type not found in package - skip validation but warn
        result.add_warning(ValidationError::warning(
            file_path,
            &format!("{}[{}].element_type", field_name, index),
            codes::UNKNOWN_ELEMENT_TYPE_REFERENCE,
            format!(
                "ElementType '{}' is not defined in this package. Skipping content validation.",
                elem_type
            ),
        ));
    }
}

/// Validate archetype content against resolved archetype fields
pub(crate) fn validate_archetype_content(
    properties: Option<&serde_yaml::Mapping>,
    archetype_name: &str,
    ctx: &ValidationContext,
    file_path: &str,
    result: &mut ValidationResult,
) {
    // Resolve all fields including inherited ones
    let mut visited = HashSet::new();
    let resolved_fields = resolve_archetype_fields(archetype_name, ctx, &mut visited);

    if let Some(props) = properties {
        validate_fields_against_content(&resolved_fields, props, archetype_name, ctx, file_path, result);
    } else {
        // No properties but have required fields
        for field in &resolved_fields {
            let field_name = field.base_name();
            let is_required = get_field_required(field);

            if is_required {
                result.add_error(ValidationError::error(
                    file_path,
                    archetype_name,
                    codes::MISSING_REQUIRED_ARCHETYPE_FIELD,
                    format!(
                        "Missing required field '{}' for archetype '{}'",
                        field_name, archetype_name
                    ),
                ));
            }
        }
    }
}
