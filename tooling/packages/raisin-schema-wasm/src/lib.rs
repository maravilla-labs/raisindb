//! WASM bindings for RaisinDB schema validation
//!
//! This module provides WASM-compatible validation functions for RaisinDB package files:
//! - manifest.yaml - Package metadata
//! - nodetypes/*.yaml - Node type definitions
//! - workspaces/*.yaml - Workspace configurations
//! - content/**/*.yaml - Content nodes

mod builtin_types;
mod errors;
mod fixes;
mod validation;

use errors::{FileType, ValidationError, ValidationResult};
use raisin_models::nodes::types::element::element_type::ElementType;
use raisin_models::nodes::types::Archetype;
use serde::Serialize;
use std::collections::HashMap;
use validation::ValidationContext;
use wasm_bindgen::prelude::*;

/// Initialize the WASM module (sets up panic hooks for better error messages)
#[wasm_bindgen]
pub fn init() {
    console_error_panic_hook::set_once();
}

/// Get list of built-in node type names
#[wasm_bindgen]
pub fn get_builtin_node_types() -> JsValue {
    let types = builtin_types::get_builtin_node_types();
    serde_wasm_bindgen::to_value(&types).unwrap_or(JsValue::NULL)
}

/// Get list of built-in workspace names
#[wasm_bindgen]
pub fn get_builtin_workspaces() -> JsValue {
    let workspaces = builtin_types::get_builtin_workspaces();
    serde_wasm_bindgen::to_value(&workspaces).unwrap_or(JsValue::NULL)
}

/// Validate a manifest.yaml file
///
/// # Arguments
/// * `yaml` - The YAML content as a string
/// * `file_path` - The file path for error reporting
///
/// # Returns
/// A ValidationResult as a JsValue
#[wasm_bindgen]
pub fn validate_manifest(yaml: &str, file_path: &str) -> JsValue {
    let result = validation::validate_manifest(yaml, file_path);
    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

/// Validate a node type YAML file
///
/// # Arguments
/// * `yaml` - The YAML content as a string
/// * `file_path` - The file path for error reporting
/// * `package_node_types` - Array of node type names defined in the package
///
/// # Returns
/// A ValidationResult as a JsValue
#[wasm_bindgen]
pub fn validate_nodetype(yaml: &str, file_path: &str, package_node_types: JsValue) -> JsValue {
    let ctx = build_context(package_node_types, JsValue::NULL);
    let result = validation::validate_nodetype(yaml, file_path, &ctx);
    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

/// Validate a workspace YAML file
///
/// # Arguments
/// * `yaml` - The YAML content as a string
/// * `file_path` - The file path for error reporting
/// * `package_node_types` - Array of node type names defined in the package
/// * `package_workspaces` - Array of workspace names defined in the package
///
/// # Returns
/// A ValidationResult as a JsValue
#[wasm_bindgen]
pub fn validate_workspace(
    yaml: &str,
    file_path: &str,
    package_node_types: JsValue,
    package_workspaces: JsValue,
) -> JsValue {
    let ctx = build_context(package_node_types, package_workspaces);
    let result = validation::validate_workspace(yaml, file_path, &ctx);
    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

/// Validate a content YAML file
///
/// # Arguments
/// * `yaml` - The YAML content as a string
/// * `file_path` - The file path for error reporting
/// * `package_node_types` - Array of node type names defined in the package
/// * `package_workspaces` - Array of workspace names defined in the package
///
/// # Returns
/// A ValidationResult as a JsValue
#[wasm_bindgen]
pub fn validate_content(
    yaml: &str,
    file_path: &str,
    package_node_types: JsValue,
    package_workspaces: JsValue,
) -> JsValue {
    let ctx = build_context(package_node_types, package_workspaces);
    let result = validation::validate_content(yaml, file_path, &ctx);
    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

/// Validate an archetype YAML file
///
/// Uses serde deserialization as the single source of truth - if it can
/// be parsed into the Archetype struct from raisin-models, it's valid.
///
/// # Arguments
/// * `yaml` - The YAML content as a string
/// * `file_path` - The file path for error reporting
///
/// # Returns
/// A ValidationResult as a JsValue
#[wasm_bindgen]
pub fn validate_archetype(yaml: &str, file_path: &str) -> JsValue {
    let ctx = ValidationContext::default();
    let result = validation::validate_archetype(yaml, file_path, &ctx);
    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

/// Validate an element type YAML file
///
/// Uses serde deserialization as the single source of truth - if it can
/// be parsed into the ElementType struct from raisin-models, it's valid.
///
/// # Arguments
/// * `yaml` - The YAML content as a string
/// * `file_path` - The file path for error reporting
///
/// # Returns
/// A ValidationResult as a JsValue
#[wasm_bindgen]
pub fn validate_elementtype(yaml: &str, file_path: &str) -> JsValue {
    let ctx = ValidationContext::default();
    let result = validation::validate_elementtype(yaml, file_path, &ctx);
    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

/// Validate an entire package directory
///
/// # Arguments
/// * `files` - A Map of file paths to their YAML content
///
/// # Returns
/// A Map of file paths to ValidationResults
#[wasm_bindgen]
pub fn validate_package(files: JsValue) -> JsValue {
    use wasm_bindgen::JsCast;

    // Parse the files from JS object
    let mut files_map: HashMap<String, String> = HashMap::new();

    if files.is_object() {
        // Convert to Object and iterate keys
        let obj: &js_sys::Object = files.unchecked_ref();
        let keys = js_sys::Object::keys(obj);

        for i in 0..keys.length() {
            if let Some(key) = keys.get(i).as_string() {
                if let Ok(value) = js_sys::Reflect::get(&files, &JsValue::from_str(&key)) {
                    if let Some(content) = value.as_string() {
                        files_map.insert(key, content);
                    }
                }
            }
        }
    }

    // If no files were parsed, return error
    if files_map.is_empty() {
        let mut result: HashMap<String, ValidationResult> = HashMap::new();
        let mut err_result = ValidationResult::success(FileType::Manifest);
        err_result.add_error(ValidationError::error(
            "",
            "",
            "INVALID_INPUT",
            "No files found in input or failed to parse files object",
        ));
        result.insert("_error".to_string(), err_result);
        let serializer = serde_wasm_bindgen::Serializer::new().serialize_maps_as_objects(true);
        return result.serialize(&serializer).unwrap_or(JsValue::NULL);
    }

    // First pass: collect all node types, workspaces, archetypes, and element types
    // For archetypes and element types, store full definitions for field validation
    let mut package_node_types = std::collections::HashSet::new();
    let mut package_workspaces = std::collections::HashSet::new();
    let mut package_archetypes: HashMap<String, Archetype> = HashMap::new();
    let mut package_element_types: HashMap<String, ElementType> = HashMap::new();

    for (path, content) in &files_map {
        if is_nodetype_file(path) {
            if let Ok(yaml) = serde_yaml::from_str::<serde_yaml::Value>(content) {
                if let Some(name) = yaml.get("name").and_then(|n| n.as_str()) {
                    package_node_types.insert(name.to_string());
                }
            }
        } else if is_workspace_file(path) {
            if let Ok(yaml) = serde_yaml::from_str::<serde_yaml::Value>(content) {
                if let Some(name) = yaml.get("name").and_then(|n| n.as_str()) {
                    package_workspaces.insert(name.to_string());
                }
            }
        } else if is_archetype_file(path) {
            // Store full archetype definition for field validation with inheritance
            if let Ok(archetype) = serde_yaml::from_str::<Archetype>(content) {
                package_archetypes.insert(archetype.name.clone(), archetype);
            }
        } else if is_elementtype_file(path) {
            // Store full element type definition for field validation with inheritance
            if let Ok(element_type) = serde_yaml::from_str::<ElementType>(content) {
                package_element_types.insert(element_type.name.clone(), element_type);
            }
        }
    }

    // Build validation context
    let ctx = ValidationContext {
        package_node_types,
        package_workspaces,
        package_archetypes,
        package_element_types,
    };

    // Second pass: validate all files
    let mut results: HashMap<String, ValidationResult> = HashMap::new();

    for (path, content) in &files_map {
        let result = if is_manifest_file(path) {
            validation::validate_manifest(content, path)
        } else if is_nodetype_file(path) {
            validation::validate_nodetype(content, path, &ctx)
        } else if is_workspace_file(path) {
            validation::validate_workspace(content, path, &ctx)
        } else if is_content_file(path) {
            validation::validate_content(content, path, &ctx)
        } else if is_archetype_file(path) {
            validation::validate_archetype(content, path, &ctx)
        } else if is_elementtype_file(path) {
            validation::validate_elementtype(content, path, &ctx)
        } else {
            // Skip unknown file types
            continue;
        };

        results.insert(path.clone(), result);
    }

    // Use serialize_maps_as_objects to convert HashMap to JS plain object
    let serializer = serde_wasm_bindgen::Serializer::new().serialize_maps_as_objects(true);
    results.serialize(&serializer).unwrap_or(JsValue::NULL)
}

/// Apply a fix to YAML content
///
/// # Arguments
/// * `yaml` - The YAML content as a string
/// * `error` - The ValidationError to fix
/// * `new_value` - Optional new value for NeedsInput fixes
///
/// # Returns
/// The modified YAML string or an error message
#[wasm_bindgen]
pub fn apply_fix(yaml: &str, error: JsValue, new_value: Option<String>) -> JsValue {
    let error: ValidationError = match serde_wasm_bindgen::from_value(error) {
        Ok(e) => e,
        Err(e) => {
            return serde_wasm_bindgen::to_value(&Err::<String, String>(format!(
                "Failed to parse error: {}",
                e
            )))
            .unwrap_or(JsValue::NULL);
        }
    };

    let result = fixes::apply_fix(yaml, &error, new_value.as_deref());
    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Build a ValidationContext from JsValue arrays
fn build_context(package_node_types: JsValue, package_workspaces: JsValue) -> ValidationContext {
    let node_types: Vec<String> = serde_wasm_bindgen::from_value(package_node_types)
        .unwrap_or_default();
    let workspaces: Vec<String> = serde_wasm_bindgen::from_value(package_workspaces)
        .unwrap_or_default();

    ValidationContext {
        package_node_types: node_types.into_iter().collect(),
        package_workspaces: workspaces.into_iter().collect(),
        package_archetypes: HashMap::new(),
        package_element_types: HashMap::new(),
    }
}

/// Check if a path is a manifest file
fn is_manifest_file(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower == "manifest.yaml" || lower == "manifest.yml" || lower.ends_with("/manifest.yaml") || lower.ends_with("/manifest.yml")
}

/// Check if a path is a node type file
fn is_nodetype_file(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.contains("nodetype") || lower.contains("nodetypes/")
}

/// Check if a path is a workspace file
fn is_workspace_file(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.contains("workspace") || lower.contains("workspaces/")
}

/// Check if a path is a content file
fn is_content_file(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.contains("content/")
}

/// Check if a path is an archetype file
fn is_archetype_file(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.contains("archetype") || lower.contains("archetypes/")
}

/// Check if a path is an element type file
fn is_elementtype_file(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.contains("elementtype") || lower.contains("elementtypes/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_manifest_file() {
        assert!(is_manifest_file("manifest.yaml"));
        assert!(is_manifest_file("manifest.yml"));
        assert!(is_manifest_file("foo/manifest.yaml"));
        assert!(!is_manifest_file("nodetypes/foo.yaml"));
    }

    #[test]
    fn test_is_nodetype_file() {
        assert!(is_nodetype_file("nodetypes/foo.yaml"));
        assert!(is_nodetype_file("nodetypes/bar.yml"));
        assert!(!is_nodetype_file("manifest.yaml"));
    }
}
