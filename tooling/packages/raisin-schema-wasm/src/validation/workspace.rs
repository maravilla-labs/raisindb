//! Workspace file validation

use crate::errors::{codes, FileType, ValidationError, ValidationResult};
use serde_yaml::Value;

use super::context::{ValidationContext, NODE_TYPE_NAME_REGEX};

/// Validate a workspace YAML file
pub fn validate_workspace(yaml_str: &str, file_path: &str, ctx: &ValidationContext) -> ValidationResult {
    let mut result = ValidationResult::success(FileType::Workspace);

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
                "Workspace must be a YAML object",
            ));
            return result;
        }
    };

    validate_name(map, file_path, &mut result);
    validate_allowed_node_types(map, file_path, ctx, &mut result);
    validate_allowed_root_node_types(map, file_path, ctx, &mut result);
    validate_depends_on(map, file_path, ctx, &mut result);

    // Optional: initial_structure - validate node types in structure
    if let Some(structure) = map.get(&Value::String("initial_structure".to_string())) {
        validate_initial_structure(structure, file_path, "initial_structure", ctx, &mut result);
    }

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
            // Workspace names can be simple identifiers or namespaced (raisin:access_control)
            if name.is_empty() {
                result.add_error(ValidationError::error(
                    file_path,
                    "name",
                    codes::INVALID_WORKSPACE_NAME,
                    "Workspace name cannot be empty",
                ));
            }
        }
        Some(_) => {
            result.add_error(ValidationError::error(
                file_path,
                "name",
                codes::INVALID_WORKSPACE_NAME,
                "Workspace name must be a string",
            ));
        }
        None => {
            result.add_error(ValidationError::error(
                file_path,
                "name",
                codes::MISSING_REQUIRED_FIELD,
                "Workspace must have a 'name' field",
            ));
        }
    }
}

/// Validate the required 'allowed_node_types' field
fn validate_allowed_node_types(
    map: &serde_yaml::Mapping,
    file_path: &str,
    ctx: &ValidationContext,
    result: &mut ValidationResult,
) {
    match map.get(&Value::String("allowed_node_types".to_string())) {
        Some(Value::Sequence(list)) => {
            validate_node_type_list(list, file_path, "allowed_node_types", ctx, result);
        }
        Some(_) => {
            result.add_error(ValidationError::error(
                file_path,
                "allowed_node_types",
                codes::YAML_PARSE_ERROR,
                "allowed_node_types must be an array",
            ));
        }
        None => {
            result.add_error(ValidationError::error(
                file_path,
                "allowed_node_types",
                codes::MISSING_REQUIRED_FIELD,
                "Workspace must have an 'allowed_node_types' field",
            ));
        }
    }
}

/// Validate the required 'allowed_root_node_types' field
fn validate_allowed_root_node_types(
    map: &serde_yaml::Mapping,
    file_path: &str,
    ctx: &ValidationContext,
    result: &mut ValidationResult,
) {
    match map.get(&Value::String("allowed_root_node_types".to_string())) {
        Some(Value::Sequence(list)) => {
            validate_node_type_list(list, file_path, "allowed_root_node_types", ctx, result);
        }
        Some(_) => {
            result.add_error(ValidationError::error(
                file_path,
                "allowed_root_node_types",
                codes::YAML_PARSE_ERROR,
                "allowed_root_node_types must be an array",
            ));
        }
        None => {
            result.add_error(ValidationError::error(
                file_path,
                "allowed_root_node_types",
                codes::MISSING_REQUIRED_FIELD,
                "Workspace must have an 'allowed_root_node_types' field",
            ));
        }
    }
}

/// Validate a list of node type references
fn validate_node_type_list(
    list: &[Value],
    file_path: &str,
    field_name: &str,
    ctx: &ValidationContext,
    result: &mut ValidationResult,
) {
    for (i, item) in list.iter().enumerate() {
        if let Value::String(type_name) = item {
            if !NODE_TYPE_NAME_REGEX.is_match(type_name) {
                result.add_error(ValidationError::error(
                    file_path,
                    &format!("{}[{}]", field_name, i),
                    codes::INVALID_ALLOWED_TYPE,
                    format!("Invalid node type '{}' in {}", type_name, field_name),
                ));
            } else if !ctx.is_valid_node_type_ref(type_name) {
                result.add_warning(ValidationError::warning(
                    file_path,
                    &format!("{}[{}]", field_name, i),
                    codes::UNKNOWN_NODE_TYPE_REFERENCE,
                    format!(
                        "NodeType '{}' in {} is not a built-in type and not defined in this package",
                        type_name, field_name
                    ),
                ));
            }
        }
    }
}

/// Validate the optional 'depends_on' field
fn validate_depends_on(
    map: &serde_yaml::Mapping,
    file_path: &str,
    ctx: &ValidationContext,
    result: &mut ValidationResult,
) {
    if let Some(Value::Sequence(list)) = map.get(&Value::String("depends_on".to_string())) {
        for (i, item) in list.iter().enumerate() {
            if let Value::String(ws_name) = item {
                if !ctx.is_valid_workspace_ref(ws_name) {
                    result.add_warning(ValidationError::warning(
                        file_path,
                        &format!("depends_on[{}]", i),
                        codes::UNKNOWN_WORKSPACE_REFERENCE,
                        format!(
                            "Workspace '{}' in depends_on is not a built-in workspace and not defined in this package",
                            ws_name
                        ),
                    ));
                }
            }
        }
    }
}

/// Validate initial_structure recursively
fn validate_initial_structure(
    structure: &Value,
    file_path: &str,
    path: &str,
    ctx: &ValidationContext,
    result: &mut ValidationResult,
) {
    let map = match structure.as_mapping() {
        Some(m) => m,
        None => return,
    };

    // Check children array
    if let Some(Value::Sequence(children)) = map.get(&Value::String("children".to_string())) {
        for (i, child) in children.iter().enumerate() {
            let child_path = format!("{}.children[{}]", path, i);

            if let Some(child_map) = child.as_mapping() {
                // Check node_type in child
                if let Some(Value::String(type_name)) =
                    child_map.get(&Value::String("node_type".to_string()))
                {
                    if !NODE_TYPE_NAME_REGEX.is_match(type_name) {
                        result.add_error(ValidationError::error(
                            file_path,
                            &format!("{}.node_type", child_path),
                            codes::INVALID_CONTENT_NODE_TYPE,
                            format!("Invalid node_type '{}' in initial_structure", type_name),
                        ));
                    } else if !ctx.is_valid_node_type_ref(type_name) {
                        result.add_warning(ValidationError::warning(
                            file_path,
                            &format!("{}.node_type", child_path),
                            codes::UNKNOWN_NODE_TYPE_REFERENCE,
                            format!(
                                "NodeType '{}' in initial_structure is not a built-in type and not defined in this package",
                                type_name
                            ),
                        ));
                    }
                }

                // Recursively validate nested children
                if child_map.contains_key(&Value::String("children".to_string())) {
                    validate_initial_structure(child, file_path, &child_path, ctx, result);
                }
            }
        }
    }
}
