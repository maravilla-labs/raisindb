//! Manifest file validation

use crate::errors::{codes, FileType, ValidationError, ValidationResult};
use serde_yaml::Value;

use super::context::PACKAGE_NAME_REGEX;
use super::helpers::sanitize_package_name;

/// Validate a manifest.yaml file
pub fn validate_manifest(yaml_str: &str, file_path: &str) -> ValidationResult {
    let mut result = ValidationResult::success(FileType::Manifest);

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

    // Must be a mapping
    let map = match yaml.as_mapping() {
        Some(m) => m,
        None => {
            result.add_error(ValidationError::error(
                file_path,
                "",
                codes::YAML_PARSE_ERROR,
                "Manifest must be a YAML object",
            ));
            return result;
        }
    };

    validate_name(map, file_path, &mut result);
    validate_version(map, file_path, &mut result);

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
            if !PACKAGE_NAME_REGEX.is_match(name) {
                result.add_error(
                    ValidationError::error(
                        file_path,
                        "name",
                        codes::INVALID_PACKAGE_NAME,
                        format!(
                            "Package name '{}' is invalid. Must start with a letter and contain only alphanumeric characters, hyphens, and underscores",
                            name
                        ),
                    )
                    .with_auto_fix_replace(
                        "Convert to valid package name",
                        name,
                        &sanitize_package_name(name),
                    ),
                );
            }
        }
        Some(_) => {
            result.add_error(ValidationError::error(
                file_path,
                "name",
                codes::INVALID_PACKAGE_NAME,
                "Package name must be a string",
            ));
        }
        None => {
            result.add_error(
                ValidationError::error(
                    file_path,
                    "name",
                    codes::MISSING_REQUIRED_FIELD,
                    "Manifest must have a 'name' field",
                )
                .with_options("Enter package name".to_string(), vec![]),
            );
        }
    }
}

/// Validate the required 'version' field
fn validate_version(
    map: &serde_yaml::Mapping,
    file_path: &str,
    result: &mut ValidationResult,
) {
    match map.get(&Value::String("version".to_string())) {
        Some(Value::String(_version)) => {
            // TODO: Validate semver format
        }
        Some(_) => {
            result.add_error(ValidationError::error(
                file_path,
                "version",
                codes::INVALID_VERSION,
                "Package version must be a string",
            ));
        }
        None => {
            result.add_error(
                ValidationError::error(
                    file_path,
                    "version",
                    codes::MISSING_REQUIRED_FIELD,
                    "Manifest must have a 'version' field",
                )
                .with_auto_fix("Add default version", "1.0.0"),
            );
        }
    }
}
