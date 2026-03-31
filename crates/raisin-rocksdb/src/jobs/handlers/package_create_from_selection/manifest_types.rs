//! Internal manifest types for .rap package format

use super::types::CollectedNode;
use serde::{Deserialize, Serialize};

/// Package manifest structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PackageManifest {
    pub name: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keywords: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provides: Option<PackageProvides>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<Vec<PackageDependency>>,
}

impl PackageManifest {
    /// Build a manifest from collected nodes
    pub fn build(
        package_name: &str,
        package_version: &str,
        content_nodes: &[CollectedNode],
        node_type_nodes: &[CollectedNode],
    ) -> Self {
        // Group content by workspace
        let mut workspaces: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut content_paths: Vec<String> = Vec::new();

        for collected in content_nodes {
            workspaces.insert(collected.workspace.clone());
            content_paths.push(format!("{}:{}", collected.workspace, collected.node.path));
        }

        let node_types: Vec<String> = node_type_nodes
            .iter()
            .map(|c| c.node.name.clone())
            .collect();

        let provides = if !workspaces.is_empty() || !node_types.is_empty() {
            Some(PackageProvides {
                nodetypes: if node_types.is_empty() {
                    None
                } else {
                    Some(node_types)
                },
                workspaces: if workspaces.is_empty() {
                    None
                } else {
                    Some(workspaces.into_iter().collect())
                },
                content: if content_paths.is_empty() {
                    None
                } else {
                    Some(content_paths)
                },
            })
        } else {
            None
        };

        PackageManifest {
            name: package_name.to_string(),
            version: package_version.to_string(),
            title: None,
            description: Some("Package created from selected content".to_string()),
            author: None,
            license: None,
            icon: None,
            color: None,
            keywords: None,
            category: None,
            provides,
            dependencies: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PackageProvides {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nodetypes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspaces: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PackageDependency {
    pub name: String,
    pub version: String,
}
