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
    validate_post_install(map, file_path, &mut result);

    result
}

/// Mirrors `raisin_packages::MAX_POST_INSTALL_HOOKS` (this crate is built
/// standalone for WASM and cannot depend on it).
const MAX_POST_INSTALL_HOOKS: usize = 32;

/// Validate the optional `post_install` list: `[{ function, input? }]`, the
/// same rules `raisin_packages::validate_post_install` applies on upload and
/// install.
fn validate_post_install(
    map: &serde_yaml::Mapping,
    file_path: &str,
    result: &mut ValidationResult,
) {
    let Some(value) = map.get(Value::String("post_install".to_string())) else {
        return;
    };
    let mut err = |field: String, message: String| {
        result.add_error(ValidationError::error(
            file_path,
            &field,
            codes::INVALID_POST_INSTALL,
            message,
        ));
    };
    let Some(entries) = value.as_sequence() else {
        err(
            "post_install".into(),
            "post_install must be a list of { function, input } entries".into(),
        );
        return;
    };
    if entries.len() > MAX_POST_INSTALL_HOOKS {
        err(
            "post_install".into(),
            format!(
                "post_install declares {} entries; at most {} are allowed",
                entries.len(),
                MAX_POST_INSTALL_HOOKS
            ),
        );
    }
    for (i, entry) in entries.iter().enumerate() {
        let field = format!("post_install[{i}]");
        let Some(obj) = entry.as_mapping() else {
            err(field, "each post_install entry must be an object".into());
            continue;
        };
        for key in obj.keys() {
            match key.as_str() {
                Some("function") | Some("input") => {}
                other => err(
                    field.clone(),
                    format!(
                        "unknown key '{}' (allowed: function, input)",
                        other.unwrap_or("?")
                    ),
                ),
            }
        }
        match obj.get(Value::String("function".to_string())) {
            Some(Value::String(path)) => {
                if path.trim().is_empty() {
                    err(
                        format!("{field}.function"),
                        "function must not be empty".into(),
                    );
                } else if path.trim() != path {
                    err(
                        format!("{field}.function"),
                        format!("function '{path}' must not have surrounding whitespace"),
                    );
                } else if !path.starts_with('/') {
                    err(
                        format!("{field}.function"),
                        format!(
                            "function '{path}' must be an absolute path in the functions \
                             workspace (starting with '/')"
                        ),
                    );
                } else if path.split('/').any(|seg| seg == "." || seg == "..") {
                    err(
                        format!("{field}.function"),
                        format!("function '{path}' must not contain '.' or '..' segments"),
                    );
                }
            }
            Some(_) => err(
                format!("{field}.function"),
                "function must be a string".into(),
            ),
            None => err(format!("{field}.function"), "function is required".into()),
        }
        if let Some(input) = obj.get(Value::String("input".to_string())) {
            if !input.is_mapping() {
                err(format!("{field}.input"), "input must be an object".into());
            }
        }
    }
}

/// Validate the required 'name' field
fn validate_name(map: &serde_yaml::Mapping, file_path: &str, result: &mut ValidationResult) {
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
fn validate_version(map: &serde_yaml::Mapping, file_path: &str, result: &mut ValidationResult) {
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

#[cfg(test)]
mod post_install_tests {
    use super::*;

    fn codes_of(yaml: &str) -> Vec<String> {
        validate_manifest(yaml, "manifest.yaml")
            .errors
            .iter()
            .map(|e| format!("{}: {}", e.field_path, e.message))
            .collect()
    }

    #[test]
    fn accepts_a_valid_hook_list() {
        let errs = codes_of(
            "name: site\nversion: 1.0.0\npost_install:\n  - { function: /lib/a, input: { x: 1 } }\n  - function: /lib/b\n",
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn rejects_bad_entries() {
        let errs = codes_of(
            "name: site\nversion: 1.0.0\npost_install:\n  - function: lib/a\n  - { function: /lib/b, input: [1] }\n  - { fn: /lib/c }\n",
        );
        assert!(
            errs.iter()
                .any(|e| e.contains("post_install[0].function") && e.contains("absolute")),
            "{errs:?}"
        );
        assert!(
            errs.iter().any(|e| e.contains("post_install[1].input")),
            "{errs:?}"
        );
        assert!(
            errs.iter().any(|e| e.contains("unknown key 'fn'")),
            "{errs:?}"
        );
        assert!(
            errs.iter().any(|e| e.contains("function is required")),
            "{errs:?}"
        );
    }

    #[test]
    fn rejects_a_non_list() {
        let errs = codes_of("name: site\nversion: 1.0.0\npost_install: /lib/a\n");
        assert_eq!(errs.len(), 1, "{errs:?}");
    }
}
