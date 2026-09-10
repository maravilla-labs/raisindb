//! Package migration file validation.

use crate::errors::{codes, FileType, ValidationError, ValidationResult};
use serde_yaml::Value;

const OPERATIONS: &[&str] = &["replace_node_type", "patch_nodes", "move_node", "delete_node"];

pub fn validate_migration(yaml_str: &str, file_path: &str) -> ValidationResult {
    let mut result = ValidationResult::success(FileType::Migration);

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

    let Some(map) = yaml.as_mapping() else {
        result.add_error(ValidationError::error(
            file_path,
            "",
            codes::YAML_PARSE_ERROR,
            "Migration must be a YAML object",
        ));
        return result;
    };

    require_string(map, file_path, &mut result, "id");

    let operations = match map
        .get(Value::String("operations".to_string()))
        .and_then(|v| v.as_sequence())
    {
        Some(ops) if !ops.is_empty() => ops,
        Some(_) => {
            result.add_error(ValidationError::error(
                file_path,
                "operations",
                codes::MISSING_REQUIRED_FIELD,
                "Migration operations must not be empty",
            ));
            return result;
        }
        None => {
            result.add_error(ValidationError::error(
                file_path,
                "operations",
                codes::MISSING_REQUIRED_FIELD,
                "Migration must have an operations list",
            ));
            return result;
        }
    };

    for (index, op) in operations.iter().enumerate() {
        let field = format!("operations[{index}]");
        let Some(op_map) = op.as_mapping() else {
            result.add_error(ValidationError::error(
                file_path,
                &field,
                codes::INVALID_MIGRATION_OPERATION,
                "Migration operation must be an object with one operation key",
            ));
            continue;
        };
        if op_map.len() != 1 {
            result.add_error(ValidationError::error(
                file_path,
                &field,
                codes::INVALID_MIGRATION_OPERATION,
                "Migration operation must contain exactly one operation key",
            ));
            continue;
        }
        let Some((name, body)) = op_map.iter().next() else {
            continue;
        };
        let Some(name) = name.as_str() else {
            result.add_error(ValidationError::error(
                file_path,
                &field,
                codes::INVALID_MIGRATION_OPERATION,
                "Migration operation name must be a string",
            ));
            continue;
        };
        if !OPERATIONS.contains(&name) {
            result.add_error(ValidationError::error(
                file_path,
                &field,
                codes::INVALID_MIGRATION_OPERATION,
                format!("Unknown migration operation '{}'", name),
            ));
            continue;
        }
        let Some(body_map) = body.as_mapping() else {
            result.add_error(ValidationError::error(
                file_path,
                format!("{field}.{name}"),
                codes::YAML_PARSE_ERROR,
                "Migration operation body must be an object",
            ));
            continue;
        };
        match name {
            "replace_node_type" => {
                for key in ["workspace", "from", "to"] {
                    require_string(body_map, file_path, &mut result, &format!("{field}.{name}.{key}"));
                }
            }
            "patch_nodes" => {
                require_string(body_map, file_path, &mut result, &format!("{field}.{name}.workspace"));
                if !body_map.contains_key(Value::String("path".to_string()))
                    && !body_map.contains_key(Value::String("node_type".to_string()))
                {
                    result.add_warning(ValidationError::warning(
                        file_path,
                        format!("{field}.{name}"),
                        codes::BROAD_MIGRATION_PATCH,
                        "patch_nodes has no path or node_type selector and will patch every node in the workspace",
                    ));
                }
            }
            "move_node" => {
                for key in ["workspace", "from", "to"] {
                    require_string(body_map, file_path, &mut result, &format!("{field}.{name}.{key}"));
                }
            }
            "delete_node" => {
                for key in ["workspace", "path"] {
                    require_string(body_map, file_path, &mut result, &format!("{field}.{name}.{key}"));
                }
            }
            _ => {}
        }
    }

    result
}

fn require_string(
    map: &serde_yaml::Mapping,
    file_path: &str,
    result: &mut ValidationResult,
    key: &str,
) {
    let leaf = key.rsplit('.').next().unwrap_or(key);
    match map.get(Value::String(leaf.to_string())) {
        Some(Value::String(s)) if !s.trim().is_empty() => {}
        Some(_) => result.add_error(ValidationError::error(
            file_path,
            key,
            codes::MISSING_REQUIRED_FIELD,
            format!("'{}' must be a non-empty string", leaf),
        )),
        None => result.add_error(ValidationError::error(
            file_path,
            key,
            codes::MISSING_REQUIRED_FIELD,
            format!("Missing required field '{}'", leaf),
        )),
    }
}

