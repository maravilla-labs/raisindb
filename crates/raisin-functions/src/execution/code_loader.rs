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

//! Function code loading from storage and binary storage.
//!
//! This module handles loading function definitions and their code from
//! the storage system and binary storage.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use raisin_binary::BinaryStorage;
use raisin_error::Result;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::{ListOptions, NodeRepository, Storage, StorageScope};

use crate::types::{FunctionLanguage, FunctionMetadata, NetworkPolicy, ResourceLimits};

/// Load a function node from the functions workspace by exact path.
pub async fn load_function_node<S>(
    storage: &S,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    functions_workspace: &str,
    function_path: &str,
) -> Result<Node>
where
    S: Storage,
{
    storage
        .nodes()
        .get_by_path(
            StorageScope::new(tenant_id, repo_id, branch, functions_workspace),
            function_path,
            None,
        )
        .await?
        .ok_or_else(|| {
            raisin_error::Error::NotFound(format!("Function not found: {}", function_path))
        })
}

/// Find a function node by name or path.
///
/// Tries direct `get_by_path` first (fast, O(1) lookup), then falls back to
/// `list_by_type("raisin:Function")` + filter by name/path (slower, scans all functions).
pub async fn find_function<S>(
    storage: &S,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    functions_workspace: &str,
    name_or_path: &str,
) -> Result<Node>
where
    S: Storage,
{
    let scope = StorageScope::new(tenant_id, repo_id, branch, functions_workspace);

    // Fast path: try direct lookup by path
    if name_or_path.starts_with('/') {
        if let Some(node) = storage
            .nodes()
            .get_by_path(scope, name_or_path, None)
            .await?
        {
            if node.node_type == "raisin:Function" {
                return Ok(node);
            }
        }
    }

    // Slow path: list all functions and filter
    let nodes = storage
        .nodes()
        .list_by_type(scope, "raisin:Function", Default::default())
        .await?;

    nodes
        .into_iter()
        .find(|n| {
            n.path == name_or_path
                || n.name == name_or_path
                || n.properties
                    .get("name")
                    .and_then(|v| match v {
                        PropertyValue::String(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .map(|p| p == name_or_path)
                    .unwrap_or(false)
        })
        .ok_or_else(|| {
            raisin_error::Error::NotFound(format!("Function '{}' not found", name_or_path))
        })
}

/// Load function code and metadata from a function node.
///
/// This function:
/// 1. Extracts the entry_file property from the function node
/// 2. Resolves the asset path relative to the function path
/// 3. Loads the asset node containing the code
/// 4. Extracts the code from inline property or binary storage
/// 5. Returns the code and function metadata
pub async fn load_function_code<S, B>(
    storage: &S,
    binary_storage: &B,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    functions_workspace: &str,
    func_node: &Node,
    function_path: &str,
) -> Result<(String, FunctionMetadata)>
where
    S: Storage,
    B: BinaryStorage,
{
    // Extract function properties
    let name = extract_name(func_node)?;
    let language = extract_language(func_node)?;
    let entry_file = extract_entry_file(func_node)?;
    let network_policy = extract_network_policy(func_node);
    let resource_limits = extract_resource_limits(func_node);

    // Resolve entry file path
    let (asset_path, handler_name) = resolve_entry_file(function_path, &entry_file);

    // Load asset node
    let asset_node = storage
        .nodes()
        .get_by_path(
            StorageScope::new(tenant_id, repo_id, branch, functions_workspace),
            &asset_path,
            None,
        )
        .await?
        .ok_or_else(|| {
            raisin_error::Error::NotFound(format!("Entry file not found: {}", asset_path))
        })?;

    // Load code from asset
    let code = extract_code_from_asset(&asset_node, binary_storage).await?;

    // Build metadata with network_policy and resource_limits from function node
    let mut metadata = FunctionMetadata::new(name, language).with_entry_file(handler_name);
    metadata.network_policy = network_policy;
    metadata.resource_limits = resource_limits;

    Ok((code, metadata))
}

/// Extract function name from node properties.
pub fn extract_name(node: &Node) -> Result<String> {
    node.properties
        .get("name")
        .and_then(|v| match v {
            PropertyValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .or_else(|| Some(node.name.clone()))
        .ok_or_else(|| {
            raisin_error::Error::Validation("Function node missing 'name' property".to_string())
        })
}

/// Extract function language from node properties.
pub fn extract_language(node: &Node) -> Result<FunctionLanguage> {
    let lang_str = node
        .properties
        .get("language")
        .and_then(|v| match v {
            PropertyValue::String(s) => Some(s.as_str()),
            _ => None,
        })
        .unwrap_or("JavaScript");

    match lang_str.to_lowercase().as_str() {
        "javascript" | "js" => Ok(FunctionLanguage::JavaScript),
        "starlark" | "python" => Ok(FunctionLanguage::Starlark),
        _ => Err(raisin_error::Error::Validation(format!(
            "Unsupported function language: {}",
            lang_str
        ))),
    }
}

/// Extract entry_file from node properties.
pub fn extract_entry_file(node: &Node) -> Result<String> {
    node.properties
        .get("entry_file")
        .and_then(|v| match v {
            PropertyValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .ok_or_else(|| {
            raisin_error::Error::Validation(
                "Function node missing 'entry_file' property".to_string(),
            )
        })
}

/// Extract network_policy from node properties.
///
/// Returns the function's network policy settings including http_enabled and allowed_urls.
/// If not specified, returns default (restrictive) policy.
pub fn extract_network_policy(node: &Node) -> NetworkPolicy {
    node.properties
        .get("network_policy")
        .and_then(|v| serde_json::to_value(v).ok())
        .and_then(|v| serde_json::from_value::<NetworkPolicy>(v).ok())
        .unwrap_or_default()
}

/// Extract resource_limits from node properties.
///
/// Returns the function's resource limits including timeout_ms, max_memory_bytes, etc.
/// If not specified, returns default limits.
pub fn extract_resource_limits(node: &Node) -> ResourceLimits {
    node.properties
        .get("resource_limits")
        .and_then(|v| serde_json::to_value(v).ok())
        .and_then(|v| serde_json::from_value::<ResourceLimits>(v).ok())
        .unwrap_or_default()
}

/// Resolve entry file path relative to function path.
///
/// Entry file format: `"filename:functionName"` (e.g., `"index.js:handleUserMessage"`)
///
/// Returns: `(full_asset_path, handler_function_name)`
pub fn resolve_entry_file(function_path: &str, entry_file: &str) -> (String, String) {
    // Split "filename:functionName" format
    let (file_part, handler) = if let Some(idx) = entry_file.rfind(':') {
        let (file, func) = entry_file.split_at(idx);
        (file.to_string(), func[1..].to_string()) // Skip the ':'
    } else {
        // No function name specified, use default "handler"
        (entry_file.to_string(), "handler".to_string())
    };

    // Build full path using std::path for normalization
    let base = Path::new(function_path);
    let file = Path::new(&file_part);

    // Join and normalize the path
    let joined = base.join(file);
    let normalized = normalize_path(&joined);

    // Convert back to string, ensuring leading /
    let path_str = normalized.to_string_lossy().to_string();
    let full_path = if path_str.starts_with('/') {
        path_str
    } else {
        format!("/{}", path_str)
    };

    (full_path, handler)
}

/// Normalize a path by resolving `.` and `..` components.
fn normalize_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();

    for component in path.components() {
        match component {
            Component::ParentDir => {
                result.pop();
            }
            Component::CurDir => {
                // Skip current directory markers
            }
            Component::RootDir => {
                result.push("/");
            }
            Component::Normal(name) => {
                result.push(name);
            }
            Component::Prefix(prefix) => {
                result.push(prefix.as_os_str());
            }
        }
    }

    result
}

/// Extract code from an asset node.
///
/// Tries inline "code" property first, then falls back to binary storage.
pub async fn extract_code_from_asset<B>(node: &Node, binary_storage: &B) -> Result<String>
where
    B: BinaryStorage,
{
    // Try inline code property first
    if let Some(PropertyValue::String(code)) = node.properties.get("code") {
        return Ok(code.clone());
    }

    // Try binary storage via "file" resource property
    if let Some(PropertyValue::Resource(res)) = node.properties.get("file") {
        if let Some(meta) = &res.metadata {
            if let Some(PropertyValue::String(storage_key)) = meta.get("storage_key") {
                let bytes = binary_storage.get(storage_key).await.map_err(|e| {
                    raisin_error::Error::storage(format!(
                        "Failed to load code from binary storage (key={}): {}",
                        storage_key, e
                    ))
                })?;

                return String::from_utf8(bytes.to_vec()).map_err(|e| {
                    raisin_error::Error::Validation(format!("Code is not valid UTF-8: {}", e))
                });
            }
        }
    }

    Err(raisin_error::Error::Validation(
        "Asset node has neither 'code' property nor 'file' resource with storage_key".to_string(),
    ))
}

/// Load sibling files for module resolution.
///
/// Lists children of the function node directory and loads code-like sibling files
/// other than the entry file. Used by:
/// - QuickJS ES modules (`.js`, `.mjs`)
/// - Starlark load() (`.py`, `.star`, `.bzl`)
/// Returns a map of relative filename to source code.
pub async fn load_sibling_files<S, B>(
    storage: &S,
    binary_storage: &B,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    functions_workspace: &str,
    function_path: &str,
    entry_file_name: &str,
) -> Result<HashMap<String, String>>
where
    S: Storage,
    B: BinaryStorage,
{
    let path_prefix = if function_path.ends_with('/') {
        function_path.to_string()
    } else {
        format!("{}/", function_path)
    };

    let descendants = storage
        .nodes()
        .scan_by_path_prefix(
            StorageScope::new(tenant_id, repo_id, branch, functions_workspace),
            &path_prefix,
            ListOptions::default(),
        )
        .await?;

    let mut files = HashMap::new();

    for child in &descendants {
        let relative_path = child
            .path
            .strip_prefix(&path_prefix)
            .unwrap_or(child.name.as_str())
            .to_string();

        let name = relative_path.as_str();

        // Skip the entry file itself
        if name == entry_file_name {
            continue;
        }

        // Only load known module-capable code files
        if !name.ends_with(".js")
            && !name.ends_with(".mjs")
            && !name.ends_with(".py")
            && !name.ends_with(".star")
            && !name.ends_with(".bzl")
        {
            continue;
        }

        // Load the code from the child asset node
        match extract_code_from_asset(child, binary_storage).await {
            Ok(code) => {
                files.insert(relative_path, code);
            }
            Err(e) => {
                tracing::warn!(
                    file = %name,
                    error = %e,
                    "Failed to load sibling file, skipping"
                );
            }
        }
    }

    Ok(files)
}

/// Extract unique directory names referenced via `../` relative imports in JS code.
///
/// Scans import statements for patterns like `from '../dir-name/...'` and returns
/// the first path component after `../` (the directory name).
fn collect_external_import_dirs(code: &str) -> Vec<String> {
    let mut dirs = Vec::new();
    for line in code.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("import ") {
            continue;
        }
        // Extract module specifier from: import ... from 'specifier'
        let spec_start = match trimmed.rfind("from ") {
            Some(i) => i + 5,
            None => continue,
        };
        let spec = trimmed[spec_start..]
            .trim()
            .trim_matches(|c: char| c == '\'' || c == '"' || c == ';');
        if !spec.starts_with("../") {
            continue;
        }
        // Get the directory name (first segment after ../)
        let after = &spec[3..];
        if let Some(pos) = after.find('/') {
            let dir = after[..pos].to_string();
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
    }
    dirs
}

/// Load files from external sibling directories referenced by `../` imports.
///
/// Scans the entry code and all sibling files for `import ... from '../dir/...'`
/// patterns, then loads code files from those referenced directories.
///
/// The returned map uses keys like `"dir-name/file.js"` which match what the
/// QuickJS module resolver produces when resolving `../dir-name/file.js` from
/// an entry module.
pub async fn load_external_modules<S, B>(
    storage: &S,
    binary_storage: &B,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    functions_workspace: &str,
    function_path: &str,
    entry_code: &str,
    sibling_files: &HashMap<String, String>,
) -> Result<HashMap<String, String>>
where
    S: Storage,
    B: BinaryStorage,
{
    // Collect external dirs from entry code and all sibling files
    let mut external_dirs = collect_external_import_dirs(entry_code);
    for code in sibling_files.values() {
        for dir in collect_external_import_dirs(code) {
            if !external_dirs.contains(&dir) {
                external_dirs.push(dir);
            }
        }
    }

    if external_dirs.is_empty() {
        return Ok(HashMap::new());
    }

    let parent_path = Path::new(function_path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    let mut result = HashMap::new();

    for dir_name in &external_dirs {
        let dir_prefix = format!("{}/{}/", parent_path, dir_name);

        let descendants = storage
            .nodes()
            .scan_by_path_prefix(
                StorageScope::new(tenant_id, repo_id, branch, functions_workspace),
                &dir_prefix,
                ListOptions::default(),
            )
            .await?;

        for child in &descendants {
            let relative = child
                .path
                .strip_prefix(&dir_prefix)
                .unwrap_or(child.name.as_str());

            // Only load code files
            if !relative.ends_with(".js") && !relative.ends_with(".mjs") {
                continue;
            }

            // Key format: "dir_name/filename.js" to match module resolver output
            let key = format!("{}/{}", dir_name, relative);

            match extract_code_from_asset(child, binary_storage).await {
                Ok(code) => {
                    result.insert(key, code);
                }
                Err(e) => {
                    tracing::warn!(
                        file = %key,
                        error = %e,
                        "Failed to load external module, skipping"
                    );
                }
            }
        }
    }

    tracing::debug!(
        function_path = %function_path,
        external_dirs = ?external_dirs,
        files_loaded = result.len(),
        "Loaded external modules"
    );

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_entry_file_simple() {
        let (path, handler) =
            resolve_entry_file("/lib/raisin/agent-handler", "index.js:handleUserMessage");
        assert_eq!(path, "/lib/raisin/agent-handler/index.js");
        assert_eq!(handler, "handleUserMessage");
    }

    #[test]
    fn test_resolve_entry_file_no_handler() {
        let (path, handler) = resolve_entry_file("/lib/raisin/agent-handler", "main.js");
        assert_eq!(path, "/lib/raisin/agent-handler/main.js");
        assert_eq!(handler, "handler"); // Default
    }

    #[test]
    fn test_resolve_entry_file_relative_parent() {
        let (path, handler) =
            resolve_entry_file("/lib/raisin/agent-handler", "../shared/utils.js:helper");
        assert_eq!(path, "/lib/raisin/shared/utils.js");
        assert_eq!(handler, "helper");
    }

    #[test]
    fn test_resolve_entry_file_nested() {
        let (path, handler) =
            resolve_entry_file("/lib/raisin/agent-handler", "src/handlers/main.js:run");
        assert_eq!(path, "/lib/raisin/agent-handler/src/handlers/main.js");
        assert_eq!(handler, "run");
    }

    #[test]
    fn test_collect_external_import_dirs_basic() {
        let code = r#"
import { log, setContext } from '../agent-shared/logger.js';
import { buildHistoryFromChat } from '../agent-shared/history.js';
import { safeJson } from './utils.js';
"#;
        let dirs = collect_external_import_dirs(code);
        assert_eq!(dirs, vec!["agent-shared"]);
    }

    #[test]
    fn test_collect_external_import_dirs_multiple() {
        let code = r#"
import { log } from '../agent-shared/logger.js';
import { helper } from '../other-lib/helper.js';
"#;
        let dirs = collect_external_import_dirs(code);
        assert_eq!(dirs, vec!["agent-shared", "other-lib"]);
    }

    #[test]
    fn test_collect_external_import_dirs_no_externals() {
        let code = r#"
import { log } from './logger.js';
import { helper } from './helper.js';
"#;
        let dirs = collect_external_import_dirs(code);
        assert!(dirs.is_empty());
    }

    #[test]
    fn test_collect_external_import_dirs_double_quotes() {
        let code = r#"
import { log } from "../agent-shared/logger.js";
"#;
        let dirs = collect_external_import_dirs(code);
        assert_eq!(dirs, vec!["agent-shared"]);
    }
}
