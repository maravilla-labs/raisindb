//! NodeType file validation

use crate::errors::{codes, FileType, ValidationError, ValidationResult};
use serde_yaml::Value;
use std::collections::HashSet;

use super::context::{ValidationContext, NODE_TYPE_NAME_REGEX, VALID_PROPERTY_TYPES};
use super::helpers::suggest_node_type_name;

/// Validate a node type YAML file
pub fn validate_nodetype(yaml_str: &str, file_path: &str, ctx: &ValidationContext) -> ValidationResult {
    let mut result = ValidationResult::success(FileType::NodeType);

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
                "NodeType must be a YAML object",
            ));
            return result;
        }
    };

    validate_name(map, file_path, &mut result);
    validate_extends(map, file_path, ctx, &mut result);
    validate_mixins(map, file_path, ctx, &mut result);

    // Optional: properties
    if let Some(properties) = map.get(&Value::String("properties".to_string())) {
        validate_properties(properties, file_path, "properties", &mut result);
    }

    validate_allowed_children(map, file_path, ctx, &mut result);

    result
}

/// Validate the required 'name' field
fn validate_name(
    map: &serde_yaml::Mapping,
    file_path: &str,
    result: &mut ValidationResult,
) {
    match map.get(&Value::String("name".to_string())) {
        Some(Value::String(name)) => {
            if !NODE_TYPE_NAME_REGEX.is_match(name) {
                let suggested = suggest_node_type_name(name);
                result.add_error(
                    ValidationError::error(
                        file_path,
                        "name",
                        codes::INVALID_NODE_TYPE_NAME,
                        format!(
                            "NodeType name '{}' is invalid. Must be in format 'namespace:PascalCase' (e.g., 'raisin:Folder')",
                            name
                        ),
                    )
                    .with_auto_fix_replace("Convert to valid format", name, &suggested),
                );
            }
        }
        Some(_) => {
            result.add_error(ValidationError::error(
                file_path,
                "name",
                codes::INVALID_NODE_TYPE_NAME,
                "NodeType name must be a string",
            ));
        }
        None => {
            result.add_error(ValidationError::error(
                file_path,
                "name",
                codes::MISSING_REQUIRED_FIELD,
                "NodeType must have a 'name' field",
            ));
        }
    }
}

/// Validate the optional 'extends' field
fn validate_extends(
    map: &serde_yaml::Mapping,
    file_path: &str,
    ctx: &ValidationContext,
    result: &mut ValidationResult,
) {
    if let Some(extends) = map.get(&Value::String("extends".to_string())) {
        if let Value::String(extends_name) = extends {
            if !NODE_TYPE_NAME_REGEX.is_match(extends_name) {
                result.add_error(ValidationError::error(
                    file_path,
                    "extends",
                    codes::INVALID_EXTENDS,
                    format!("Invalid extends value '{}'. Must be a valid NodeType name", extends_name),
                ));
            } else if !ctx.is_valid_node_type_ref(extends_name) {
                result.add_warning(ValidationError::warning(
                    file_path,
                    "extends",
                    codes::UNKNOWN_NODE_TYPE_REFERENCE,
                    format!(
                        "NodeType '{}' referenced in 'extends' is not a built-in type and not defined in this package",
                        extends_name
                    ),
                ));
            }
        }
    }
}

/// Validate the optional 'mixins' field
fn validate_mixins(
    map: &serde_yaml::Mapping,
    file_path: &str,
    ctx: &ValidationContext,
    result: &mut ValidationResult,
) {
    if let Some(mixins) = map.get(&Value::String("mixins".to_string())) {
        if let Value::Sequence(mixin_list) = mixins {
            for (i, mixin) in mixin_list.iter().enumerate() {
                if let Value::String(mixin_name) = mixin {
                    if !NODE_TYPE_NAME_REGEX.is_match(mixin_name) {
                        result.add_error(ValidationError::error(
                            file_path,
                            &format!("mixins[{}]", i),
                            codes::INVALID_MIXIN,
                            format!("Invalid mixin '{}'. Must be a valid NodeType name", mixin_name),
                        ));
                    } else if !ctx.is_valid_node_type_ref(mixin_name) {
                        result.add_warning(ValidationError::warning(
                            file_path,
                            &format!("mixins[{}]", i),
                            codes::UNKNOWN_NODE_TYPE_REFERENCE,
                            format!(
                                "NodeType '{}' referenced in mixins is not a built-in type and not defined in this package",
                                mixin_name
                            ),
                        ));
                    }
                }
            }
        }
    }
}

/// Validate the optional 'allowed_children' field
fn validate_allowed_children(
    map: &serde_yaml::Mapping,
    file_path: &str,
    ctx: &ValidationContext,
    result: &mut ValidationResult,
) {
    if let Some(allowed) = map.get(&Value::String("allowed_children".to_string())) {
        if let Value::Sequence(list) = allowed {
            for (i, child) in list.iter().enumerate() {
                if let Value::String(child_name) = child {
                    if !NODE_TYPE_NAME_REGEX.is_match(child_name) {
                        result.add_error(ValidationError::error(
                            file_path,
                            &format!("allowed_children[{}]", i),
                            codes::INVALID_ALLOWED_TYPE,
                            format!("Invalid allowed_children value '{}'. Must be a valid NodeType name", child_name),
                        ));
                    } else if !ctx.is_valid_node_type_ref(child_name) {
                        result.add_warning(ValidationError::warning(
                            file_path,
                            &format!("allowed_children[{}]", i),
                            codes::UNKNOWN_NODE_TYPE_REFERENCE,
                            format!(
                                "NodeType '{}' in allowed_children is not a built-in type and not defined in this package",
                                child_name
                            ),
                        ));
                    }
                }
            }
        }
    }
}

/// Validate properties array
fn validate_properties(props: &Value, file_path: &str, path: &str, result: &mut ValidationResult) {
    let props_list = match props {
        Value::Sequence(list) => list,
        _ => {
            result.add_error(ValidationError::error(
                file_path,
                path,
                codes::YAML_PARSE_ERROR,
                "Properties must be an array",
            ));
            return;
        }
    };

    let mut seen_names = HashSet::new();

    for (i, prop) in props_list.iter().enumerate() {
        let prop_path = format!("{}[{}]", path, i);

        let prop_map = match prop.as_mapping() {
            Some(m) => m,
            None => {
                result.add_error(ValidationError::error(
                    file_path,
                    &prop_path,
                    codes::YAML_PARSE_ERROR,
                    "Property must be an object",
                ));
                continue;
            }
        };

        // Check name for duplicates
        if let Some(Value::String(name)) = prop_map.get(&Value::String("name".to_string())) {
            if seen_names.contains(name) {
                result.add_error(ValidationError::error(
                    file_path,
                    &format!("{}.name", prop_path),
                    codes::DUPLICATE_PROPERTY,
                    format!("Duplicate property name '{}'", name),
                ));
            } else {
                seen_names.insert(name.clone());
            }
        }

        // Required: type
        validate_property_type(prop_map, file_path, &prop_path, result);
    }
}

/// Validate a single property's 'type' field
fn validate_property_type(
    prop_map: &serde_yaml::Mapping,
    file_path: &str,
    prop_path: &str,
    result: &mut ValidationResult,
) {
    match prop_map.get(&Value::String("type".to_string())) {
        Some(Value::String(type_str)) => {
            if !VALID_PROPERTY_TYPES.contains(type_str.as_str()) {
                let valid_types: Vec<String> = VALID_PROPERTY_TYPES
                    .iter()
                    .filter(|t| t.chars().next().map(|c| c.is_uppercase()).unwrap_or(false))
                    .map(|s| s.to_string())
                    .collect();
                result.add_error(
                    ValidationError::error(
                        file_path,
                        &format!("{}.type", prop_path),
                        codes::INVALID_PROPERTY_TYPE,
                        format!("Invalid property type '{}'. Valid types: {:?}", type_str, valid_types),
                    )
                    .with_options("Select valid property type".to_string(), valid_types),
                );
            }
        }
        Some(_) => {
            result.add_error(ValidationError::error(
                file_path,
                &format!("{}.type", prop_path),
                codes::INVALID_PROPERTY_TYPE,
                "Property type must be a string",
            ));
        }
        None => {
            result.add_error(ValidationError::error(
                file_path,
                &format!("{}.type", prop_path),
                codes::MISSING_REQUIRED_FIELD,
                "Property must have a 'type' field",
            ));
        }
    }
}
