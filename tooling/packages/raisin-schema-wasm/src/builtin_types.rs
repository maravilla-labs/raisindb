//! Built-in types embedded at compile time from global_nodetypes/ and global_workspaces/

use include_dir::{include_dir, Dir};
use lazy_static::lazy_static;
use std::collections::HashSet;

// Embed the global_nodetypes and global_workspaces directories at compile time
static GLOBAL_NODETYPES_DIR: Dir<'_> =
    include_dir!("$CARGO_MANIFEST_DIR/../../../crates/raisin-core/global_nodetypes");
static GLOBAL_WORKSPACES_DIR: Dir<'_> =
    include_dir!("$CARGO_MANIFEST_DIR/../../../crates/raisin-core/global_workspaces");

lazy_static! {
    /// Set of built-in node type names (e.g., "raisin:Folder", "raisin:Page")
    pub static ref BUILTIN_NODE_TYPES: HashSet<String> = {
        let mut types = HashSet::new();

        for file in GLOBAL_NODETYPES_DIR.files() {
            if let Some(ext) = file.path().extension() {
                if ext == "yaml" || ext == "yml" {
                    if let Some(content) = file.contents_utf8() {
                        // Parse YAML to extract the name field
                        if let Ok(yaml) = serde_yaml::from_str::<serde_yaml::Value>(content) {
                            if let Some(name) = yaml.get("name").and_then(|n| n.as_str()) {
                                types.insert(name.to_string());
                            }
                        }
                    }
                }
            }
        }

        types
    };

    /// Set of built-in workspace names (e.g., "default", "functions", "access_control")
    pub static ref BUILTIN_WORKSPACES: HashSet<String> = {
        let mut workspaces = HashSet::new();

        for file in GLOBAL_WORKSPACES_DIR.files() {
            if let Some(ext) = file.path().extension() {
                if ext == "yaml" || ext == "yml" {
                    if let Some(content) = file.contents_utf8() {
                        // Parse YAML to extract the name field
                        if let Ok(yaml) = serde_yaml::from_str::<serde_yaml::Value>(content) {
                            if let Some(name) = yaml.get("name").and_then(|n| n.as_str()) {
                                workspaces.insert(name.to_string());
                            }
                        }
                    }
                }
            }
        }

        workspaces
    };
}

/// Check if a node type name is a built-in type
pub fn is_builtin_node_type(name: &str) -> bool {
    BUILTIN_NODE_TYPES.contains(name)
}

/// Check if a workspace name is a built-in workspace
pub fn is_builtin_workspace(name: &str) -> bool {
    BUILTIN_WORKSPACES.contains(name)
}

/// Get all built-in node type names
pub fn get_builtin_node_types() -> Vec<String> {
    BUILTIN_NODE_TYPES.iter().cloned().collect()
}

/// Get all built-in workspace names
pub fn get_builtin_workspaces() -> Vec<String> {
    BUILTIN_WORKSPACES.iter().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_node_types_loaded() {
        // Should have at least raisin:Folder
        assert!(BUILTIN_NODE_TYPES.contains("raisin:Folder"));
    }

    #[test]
    fn test_builtin_workspaces_loaded() {
        // Should have at least 'default' workspace
        assert!(BUILTIN_WORKSPACES.contains("default"));
    }
}
