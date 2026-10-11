//! Internal manifest types for .rap package format

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

/// What the manifest says about a finished walk.
pub(super) struct ManifestInput<'a> {
    pub name: &'a str,
    pub version: &'a str,
    pub title: Option<String>,
    pub description: Option<String>,
    pub author: Option<String>,
    /// The selection as requested (`ws:/path`, `ws:/path/*`, `ws:/*`) — not
    /// every node: that list is the inventory's job, and a manifest holding a
    /// million paths would be read by every installer in full.
    pub content: Vec<String>,
    pub workspaces: Vec<String>,
    pub node_types: Vec<String>,
}

impl PackageManifest {
    pub fn build(input: ManifestInput<'_>) -> Self {
        let provides = if input.workspaces.is_empty() && input.node_types.is_empty() {
            None
        } else {
            let some = |v: Vec<String>| (!v.is_empty()).then_some(v);
            Some(PackageProvides {
                nodetypes: some(input.node_types),
                workspaces: some(input.workspaces),
                content: some(input.content),
            })
        };

        PackageManifest {
            name: input.name.to_string(),
            version: input.version.to_string(),
            title: input.title,
            description: Some(
                input
                    .description
                    .unwrap_or_else(|| "Package created from selected content".to_string()),
            ),
            author: input.author,
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
