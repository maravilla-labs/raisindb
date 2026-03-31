//! Content file validation

use crate::errors::{codes, FileType, ValidationError, ValidationResult};
use serde_yaml::Value;

use super::context::{ValidationContext, NODE_TYPE_NAME_REGEX};
use super::field_resolution::{validate_archetype_content, validate_element_content};

/// Validate a content YAML file
pub fn validate_content(yaml_str: &str, file_path: &str, ctx: &ValidationContext) -> ValidationResult {
    let mut result = ValidationResult::success(FileType::Content);

    // Parse YAML
    let yaml: Value = match serde_yaml::from_str(yaml_str) {
        Ok(v) => v,
        Err(e) => {
            result.add_error(ValidationError::error(
                file_path,
                "",
                codes::YAML_SYNTAX_ERROR,
                format!("Failed to parse YAML: {}", e),
            ));
            return result;
        }
    };

    let map = match yaml.as_mapping() {
        Some(m) => m,
        None => {
            result.add_error(ValidationError::error(
                file_path,
                "",
                codes::YAML_PARSE_ERROR,
                "Content must be a YAML object",
            ));
            return result;
        }
    };

    validate_node_type(map, file_path, ctx, &mut result);
    validate_archetype(map, file_path, ctx, &mut result);

    // Optional: properties - validate references within
    if let Some(properties) = map.get(&Value::String("properties".to_string())) {
        validate_content_references(properties, file_path, "properties", ctx, &mut result);
    }

    result
}

/// Validate the required 'node_type' field
fn validate_node_type(
    map: &serde_yaml::Mapping,
    file_path: &str,
    ctx: &ValidationContext,
    result: &mut ValidationResult,
) {
    match map.get(&Value::String("node_type".to_string())) {
        Some(Value::String(type_name)) => {
            if !NODE_TYPE_NAME_REGEX.is_match(type_name) {
                result.add_error(ValidationError::error(
                    file_path,
                    "node_type",
                    codes::INVALID_CONTENT_NODE_TYPE,
                    format!("Invalid node_type '{}'. Must be in format 'namespace:PascalCase'", type_name),
                ));
            } else if !ctx.is_valid_node_type_ref(type_name) {
                result.add_warning(ValidationError::warning(
                    file_path,
                    "node_type",
                    codes::UNKNOWN_NODE_TYPE_REFERENCE,
                    format!(
                        "NodeType '{}' is not a built-in type and not defined in this package",
                        type_name
                    ),
                ));
            }
        }
        Some(_) => {
            result.add_error(ValidationError::error(
                file_path,
                "node_type",
                codes::INVALID_CONTENT_NODE_TYPE,
                "node_type must be a string",
            ));
        }
        None => {
            result.add_error(ValidationError::error(
                file_path,
                "node_type",
                codes::MISSING_REQUIRED_FIELD,
                "Content must have a 'node_type' field",
            ));
        }
    }
}

/// Validate the optional 'archetype' field and its required fields
fn validate_archetype(
    map: &serde_yaml::Mapping,
    file_path: &str,
    ctx: &ValidationContext,
    result: &mut ValidationResult,
) {
    if let Some(Value::String(archetype_name)) = map.get(&Value::String("archetype".to_string())) {
        if ctx.is_valid_archetype_ref(archetype_name) {
            // Validate required fields against resolved archetype (with inheritance)
            let properties_map = map
                .get(&Value::String("properties".to_string()))
                .and_then(|p| p.as_mapping());
            validate_archetype_content(properties_map, archetype_name, ctx, file_path, result);
        } else {
            result.add_warning(ValidationError::warning(
                file_path,
                "archetype",
                codes::UNKNOWN_ARCHETYPE_REFERENCE,
                format!(
                    "Archetype '{}' is not defined in this package. It may exist in the database.",
                    archetype_name
                ),
            ));
        }
    }
}

/// Validate content references (RaisinReference, Element types) within properties
fn validate_content_references(
    value: &Value,
    file_path: &str,
    path: &str,
    ctx: &ValidationContext,
    result: &mut ValidationResult,
) {
    match value {
        Value::Mapping(map) => {
            validate_mapping_references(map, file_path, path, ctx, result);
        }
        Value::Sequence(list) => {
            for (i, item) in list.iter().enumerate() {
                let item_path = format!("{}[{}]", path, i);
                validate_content_references(item, file_path, &item_path, ctx, result);
            }
        }
        _ => {}
    }
}

/// Validate references within a YAML mapping
fn validate_mapping_references(
    map: &serde_yaml::Mapping,
    file_path: &str,
    path: &str,
    ctx: &ValidationContext,
    result: &mut ValidationResult,
) {
    // Check if this is a RaisinReference
    if map.contains_key(&Value::String("raisin:ref".to_string())) {
        validate_raisin_reference(map, file_path, path, ctx, result);
        return;
    }

    // Check if this is an Element (has $type field)
    if let Some(Value::String(element_type)) = map.get(&Value::String("$type".to_string())) {
        validate_dollar_type_element(map, element_type, file_path, path, ctx, result);
        return;
    }

    // Check if this uses element_type format
    if let Some(Value::String(element_type)) = map.get(&Value::String("element_type".to_string())) {
        validate_element_type_format(map, element_type, file_path, path, ctx, result);
        return;
    }

    // Check if this is a Composite (has elements array)
    if let Some(Value::Sequence(elements)) = map.get(&Value::String("elements".to_string())) {
        for (i, element) in elements.iter().enumerate() {
            let element_path = format!("{}.elements[{}]", path, i);
            validate_content_references(element, file_path, &element_path, ctx, result);
        }
        return;
    }

    // Recursively check nested objects
    for (key, val) in map {
        if let Value::String(key_str) = key {
            let nested_path = format!("{}.{}", path, key_str);
            validate_content_references(val, file_path, &nested_path, ctx, result);
        }
    }
}

/// Validate a RaisinReference
fn validate_raisin_reference(
    map: &serde_yaml::Mapping,
    file_path: &str,
    path: &str,
    ctx: &ValidationContext,
    result: &mut ValidationResult,
) {
    if !map.contains_key(&Value::String("raisin:workspace".to_string())) {
        result.add_error(ValidationError::error(
            file_path,
            path,
            codes::UNRESOLVABLE_CONTENT_REFERENCE,
            "RaisinReference must have 'raisin:workspace' field",
        ));
    } else if let Some(Value::String(ws)) =
        map.get(&Value::String("raisin:workspace".to_string()))
    {
        if !ctx.is_valid_workspace_ref(ws) {
            result.add_warning(ValidationError::warning(
                file_path,
                &format!("{}.raisin:workspace", path),
                codes::UNKNOWN_WORKSPACE_REFERENCE,
                format!(
                    "Workspace '{}' in reference is not a built-in workspace",
                    ws
                ),
            ));
        }
    }
}

/// Validate an element with $type field
fn validate_dollar_type_element(
    map: &serde_yaml::Mapping,
    element_type: &str,
    file_path: &str,
    path: &str,
    ctx: &ValidationContext,
    result: &mut ValidationResult,
) {
    if ctx.is_valid_element_type_ref(element_type) {
        validate_element_content(map, element_type, ctx, file_path, result);
    } else {
        result.add_warning(ValidationError::warning(
            file_path,
            &format!("{}.$type", path),
            codes::UNKNOWN_ELEMENT_TYPE_REFERENCE,
            format!(
                "ElementType '{}' is not defined in this package. It may exist in the database.",
                element_type
            ),
        ));
    }
    // Recursively check nested content within element
    for (key, val) in map {
        if let Value::String(key_str) = key {
            if key_str != "$type" {
                let nested_path = format!("{}.{}", path, key_str);
                validate_content_references(val, file_path, &nested_path, ctx, result);
            }
        }
    }
}

/// Validate an element with element_type field (nested content or flat format)
fn validate_element_type_format(
    map: &serde_yaml::Mapping,
    element_type: &str,
    file_path: &str,
    path: &str,
    ctx: &ValidationContext,
    result: &mut ValidationResult,
) {
    // Check for nested content format first
    if let Some(Value::Mapping(content)) = map.get(&Value::String("content".to_string())) {
        validate_element_with_nested_content(content, element_type, file_path, path, ctx, result);
    } else {
        // Flat format: element_type + fields at same level (no content wrapper)
        validate_element_flat_format(map, element_type, file_path, path, ctx, result);
    }
}

/// Validate an element with nested content wrapper
fn validate_element_with_nested_content(
    content: &serde_yaml::Mapping,
    element_type: &str,
    file_path: &str,
    path: &str,
    ctx: &ValidationContext,
    result: &mut ValidationResult,
) {
    if ctx.is_valid_element_type_ref(element_type) {
        validate_element_content(content, element_type, ctx, file_path, result);
    } else {
        result.add_warning(ValidationError::warning(
            file_path,
            &format!("{}.element_type", path),
            codes::UNKNOWN_ELEMENT_TYPE_REFERENCE,
            format!(
                "ElementType '{}' is not defined in this package. It may exist in the database.",
                element_type
            ),
        ));
    }
    // Recursively check nested content within element's content
    for (key, val) in content {
        if let Value::String(key_str) = key {
            let nested_path = format!("{}.content.{}", path, key_str);
            validate_content_references(val, file_path, &nested_path, ctx, result);
        }
    }
}

/// Validate an element in flat format (no content wrapper)
fn validate_element_flat_format(
    map: &serde_yaml::Mapping,
    element_type: &str,
    file_path: &str,
    path: &str,
    ctx: &ValidationContext,
    result: &mut ValidationResult,
) {
    if ctx.is_valid_element_type_ref(element_type) {
        validate_element_content(map, element_type, ctx, file_path, result);
    } else {
        result.add_warning(ValidationError::warning(
            file_path,
            &format!("{}.element_type", path),
            codes::UNKNOWN_ELEMENT_TYPE_REFERENCE,
            format!(
                "ElementType '{}' is not defined in this package. It may exist in the database.",
                element_type
            ),
        ));
    }
    // Recursively check nested content within element's fields (flat format)
    for (key, val) in map {
        if let Value::String(key_str) = key {
            // Skip metadata fields
            if key_str != "element_type" && key_str != "uuid" {
                let nested_path = format!("{}.{}", path, key_str);
                validate_content_references(val, file_path, &nested_path, ctx, result);
            }
        }
    }
}
