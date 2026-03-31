// SPDX-License-Identifier: BSL-1.1

//! Workspace patching - apply patches to workspace configurations

use std::collections::HashMap;

use crate::error::{PackageError, PackageResult};
use crate::manifest::WorkspacePatch;

/// Type of patch operation
#[derive(Debug, Clone)]
pub enum PatchOperation {
    /// Add node types to allowed_node_types
    AddAllowedNodeTypes(Vec<String>),
}

/// Workspace patcher for applying package patches
pub struct WorkspacePatcher {
    patches: HashMap<String, Vec<PatchOperation>>,
}

impl WorkspacePatcher {
    /// Create a new patcher from manifest workspace_patches
    pub fn from_manifest_patches(workspace_patches: &HashMap<String, WorkspacePatch>) -> Self {
        let mut patches: HashMap<String, Vec<PatchOperation>> = HashMap::new();

        for (workspace_name, patch) in workspace_patches {
            let mut ops = Vec::new();

            // Add allowed_node_types patch
            if !patch.allowed_node_types.add.is_empty() {
                ops.push(PatchOperation::AddAllowedNodeTypes(
                    patch.allowed_node_types.add.clone(),
                ));
            }

            if !ops.is_empty() {
                patches.insert(workspace_name.clone(), ops);
            }
        }

        Self { patches }
    }

    /// Get workspaces that need patching
    pub fn workspaces_to_patch(&self) -> Vec<&str> {
        self.patches.keys().map(|s| s.as_str()).collect()
    }

    /// Get patch operations for a workspace
    pub fn get_patches(&self, workspace: &str) -> Option<&Vec<PatchOperation>> {
        self.patches.get(workspace)
    }

    /// Apply patches to a workspace configuration
    ///
    /// Takes the current workspace config as JSON and returns the patched config
    pub fn apply_patches(
        &self,
        workspace: &str,
        mut config: serde_json::Value,
    ) -> PackageResult<serde_json::Value> {
        let Some(operations) = self.patches.get(workspace) else {
            return Ok(config);
        };

        for op in operations {
            match op {
                PatchOperation::AddAllowedNodeTypes(node_types) => {
                    // Get or create allowed_node_types array
                    let allowed = config
                        .as_object_mut()
                        .ok_or_else(|| {
                            PackageError::InvalidPackage("Workspace config is not an object".into())
                        })?
                        .entry("allowed_node_types")
                        .or_insert(serde_json::json!([]));

                    let arr = allowed.as_array_mut().ok_or_else(|| {
                        PackageError::InvalidPackage("allowed_node_types is not an array".into())
                    })?;

                    // Add new node types (avoid duplicates)
                    for nt in node_types {
                        let nt_value = serde_json::Value::String(nt.clone());
                        if !arr.contains(&nt_value) {
                            arr.push(nt_value);
                        }
                    }
                }
            }
        }

        Ok(config)
    }

    /// Generate reverse patches for uninstallation
    pub fn generate_reverse_patches(&self, workspace: &str) -> Option<Vec<PatchOperation>> {
        // For now, we don't support automatic reversal
        // This would require tracking what was added vs. what was already there
        // Return None to indicate manual cleanup may be needed
        let _ = workspace;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::AllowedNodeTypesPatch;

    #[test]
    fn test_patcher_from_manifest() {
        let mut workspace_patches = HashMap::new();
        workspace_patches.insert(
            "functions".to_string(),
            WorkspacePatch {
                allowed_node_types: AllowedNodeTypesPatch {
                    add: vec!["ai:Agent".to_string(), "ai:Chat".to_string()],
                },
                default_folder_type: None,
            },
        );

        let patcher = WorkspacePatcher::from_manifest_patches(&workspace_patches);
        assert!(patcher.workspaces_to_patch().contains(&"functions"));

        let patches = patcher.get_patches("functions").unwrap();
        assert_eq!(patches.len(), 1);
    }

    #[test]
    fn test_apply_patches() {
        let mut workspace_patches = HashMap::new();
        workspace_patches.insert(
            "functions".to_string(),
            WorkspacePatch {
                allowed_node_types: AllowedNodeTypesPatch {
                    add: vec!["ai:Agent".to_string()],
                },
                default_folder_type: None,
            },
        );

        let patcher = WorkspacePatcher::from_manifest_patches(&workspace_patches);

        let config = serde_json::json!({
            "name": "functions",
            "allowed_node_types": ["raisin:Function", "raisin:Trigger"]
        });

        let patched = patcher.apply_patches("functions", config).unwrap();

        let allowed = patched["allowed_node_types"].as_array().unwrap();
        assert!(allowed.contains(&serde_json::json!("ai:Agent")));
        assert!(allowed.contains(&serde_json::json!("raisin:Function")));
    }
}
