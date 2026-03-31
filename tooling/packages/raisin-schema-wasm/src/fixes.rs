//! Auto-fix functionality for validation errors

use crate::errors::{FixType, SuggestedFix, ValidationError};
use serde_yaml::Value;

/// Apply a fix to YAML content
/// Returns the modified YAML string or an error message
pub fn apply_fix(yaml_str: &str, error: &ValidationError, new_value: Option<&str>) -> Result<String, String> {
    match error.fix_type {
        FixType::AutoFixable => apply_auto_fix(yaml_str, error),
        FixType::NeedsInput => {
            if let Some(value) = new_value {
                apply_value_fix(yaml_str, error, value)
            } else {
                Err("NeedsInput fix requires a value".to_string())
            }
        }
        FixType::Manual => Err("Manual fixes cannot be applied automatically".to_string()),
    }
}

/// Apply an auto-fixable fix
fn apply_auto_fix(yaml_str: &str, error: &ValidationError) -> Result<String, String> {
    let fix = error
        .suggested_fix
        .as_ref()
        .ok_or_else(|| "No suggested fix available".to_string())?;

    let new_value = fix
        .new_value
        .as_ref()
        .ok_or_else(|| "No new value in suggested fix".to_string())?;

    apply_value_fix(yaml_str, error, new_value)
}

/// Apply a fix by setting a value at the field path
fn apply_value_fix(yaml_str: &str, error: &ValidationError, new_value: &str) -> Result<String, String> {
    // Parse the YAML
    let mut yaml: Value =
        serde_yaml::from_str(yaml_str).map_err(|e| format!("Failed to parse YAML: {}", e))?;

    // Parse the field path and navigate to the target
    let path_parts = parse_field_path(&error.field_path);

    // Navigate to parent and set the value
    set_value_at_path(&mut yaml, &path_parts, new_value)?;

    // Serialize back to YAML
    serde_yaml::to_string(&yaml).map_err(|e| format!("Failed to serialize YAML: {}", e))
}

/// Parse a field path like "properties[0].name" into parts
fn parse_field_path(path: &str) -> Vec<PathPart> {
    let mut parts = Vec::new();
    let mut current = String::new();

    let chars: Vec<char> = path.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        if c == '.' {
            if !current.is_empty() {
                parts.push(PathPart::Key(current.clone()));
                current.clear();
            }
        } else if c == '[' {
            if !current.is_empty() {
                parts.push(PathPart::Key(current.clone()));
                current.clear();
            }
            // Read until ]
            i += 1;
            let mut index_str = String::new();
            while i < chars.len() && chars[i] != ']' {
                index_str.push(chars[i]);
                i += 1;
            }
            if let Ok(index) = index_str.parse::<usize>() {
                parts.push(PathPart::Index(index));
            }
        } else {
            current.push(c);
        }

        i += 1;
    }

    if !current.is_empty() {
        parts.push(PathPart::Key(current));
    }

    parts
}

#[derive(Debug, Clone)]
enum PathPart {
    Key(String),
    Index(usize),
}

/// Set a value at the given path in a YAML value
fn set_value_at_path(yaml: &mut Value, path: &[PathPart], new_value: &str) -> Result<(), String> {
    if path.is_empty() {
        // Replace the entire value
        *yaml = Value::String(new_value.to_string());
        return Ok(());
    }

    let mut current = yaml;

    // Navigate to parent of target
    for (i, part) in path.iter().enumerate() {
        let is_last = i == path.len() - 1;

        match part {
            PathPart::Key(key) => {
                if is_last {
                    // Set the value
                    if let Value::Mapping(map) = current {
                        map.insert(Value::String(key.clone()), Value::String(new_value.to_string()));
                        return Ok(());
                    } else {
                        return Err(format!("Cannot set key '{}' on non-mapping value", key));
                    }
                } else {
                    // Navigate deeper
                    if let Value::Mapping(map) = current {
                        current = map
                            .get_mut(&Value::String(key.clone()))
                            .ok_or_else(|| format!("Key '{}' not found", key))?;
                    } else {
                        return Err(format!("Cannot navigate key '{}' on non-mapping value", key));
                    }
                }
            }
            PathPart::Index(index) => {
                if is_last {
                    // Set the value at index
                    if let Value::Sequence(list) = current {
                        if *index < list.len() {
                            list[*index] = Value::String(new_value.to_string());
                            return Ok(());
                        } else {
                            return Err(format!("Index {} out of bounds", index));
                        }
                    } else {
                        return Err(format!("Cannot set index {} on non-sequence value", index));
                    }
                } else {
                    // Navigate deeper
                    if let Value::Sequence(list) = current {
                        current = list
                            .get_mut(*index)
                            .ok_or_else(|| format!("Index {} out of bounds", index))?;
                    } else {
                        return Err(format!("Cannot navigate index {} on non-sequence value", index));
                    }
                }
            }
        }
    }

    Ok(())
}

/// Add a missing field with a default value
pub fn add_missing_field(yaml_str: &str, field_path: &str, value: &str) -> Result<String, String> {
    let mut yaml: Value =
        serde_yaml::from_str(yaml_str).map_err(|e| format!("Failed to parse YAML: {}", e))?;

    let path_parts = parse_field_path(field_path);
    set_value_at_path(&mut yaml, &path_parts, value)?;

    serde_yaml::to_string(&yaml).map_err(|e| format!("Failed to serialize YAML: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_field_path() {
        let parts = parse_field_path("properties[0].name");
        assert_eq!(parts.len(), 3);
        matches!(&parts[0], PathPart::Key(k) if k == "properties");
        matches!(&parts[1], PathPart::Index(0));
        matches!(&parts[2], PathPart::Key(k) if k == "name");
    }

    #[test]
    fn test_apply_value_fix() {
        let yaml = r#"
name: test
version: 1.0.0
"#;
        let error = ValidationError::error("test.yaml", "name", "TEST", "test error");
        let result = apply_value_fix(yaml, &error, "new-name").unwrap();
        assert!(result.contains("new-name"));
    }
}
