//! Package manifest types and property conversion
//!
//! Contains the YAML manifest structure parsed from `.rap` packages
//! and the logic to convert manifest fields into node properties.

use raisin_models::nodes::properties::value::PropertyValue;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageManifest {
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
    pub dependencies: Option<Vec<PackageDependency>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provides: Option<PackageProvides>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_patches: Option<HashMap<String, WorkspacePatch>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageDependency {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageProvides {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nodetypes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspaces: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspacePatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_node_types: Option<AllowedNodeTypesPatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllowedNodeTypesPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub add: Option<Vec<String>>,
}

/// Build node properties from a package manifest
pub(crate) fn build_package_properties(
    manifest: &PackageManifest,
) -> HashMap<String, PropertyValue> {
    let mut properties = HashMap::new();

    properties.insert(
        "name".to_string(),
        PropertyValue::String(manifest.name.clone()),
    );
    properties.insert(
        "version".to_string(),
        PropertyValue::String(manifest.version.clone()),
    );

    if let Some(title) = &manifest.title {
        properties.insert("title".to_string(), PropertyValue::String(title.clone()));
    }
    if let Some(description) = &manifest.description {
        properties.insert(
            "description".to_string(),
            PropertyValue::String(description.clone()),
        );
    }
    if let Some(author) = &manifest.author {
        properties.insert("author".to_string(), PropertyValue::String(author.clone()));
    }
    if let Some(license) = &manifest.license {
        properties.insert(
            "license".to_string(),
            PropertyValue::String(license.clone()),
        );
    }
    if let Some(icon) = &manifest.icon {
        properties.insert("icon".to_string(), PropertyValue::String(icon.clone()));
    }
    if let Some(color) = &manifest.color {
        properties.insert("color".to_string(), PropertyValue::String(color.clone()));
    }
    if let Some(keywords) = &manifest.keywords {
        properties.insert(
            "keywords".to_string(),
            PropertyValue::Array(
                keywords
                    .iter()
                    .map(|k| PropertyValue::String(k.clone()))
                    .collect(),
            ),
        );
    }
    if let Some(category) = &manifest.category {
        properties.insert(
            "category".to_string(),
            PropertyValue::String(category.clone()),
        );
    }

    // Dependencies
    if let Some(dependencies) = &manifest.dependencies {
        let mut deps_map = HashMap::new();
        for (i, dep) in dependencies.iter().enumerate() {
            let mut dep_obj = HashMap::new();
            dep_obj.insert("name".to_string(), PropertyValue::String(dep.name.clone()));
            dep_obj.insert(
                "version".to_string(),
                PropertyValue::String(dep.version.clone()),
            );
            deps_map.insert(i.to_string(), PropertyValue::Object(dep_obj));
        }
        properties.insert("dependencies".to_string(), PropertyValue::Object(deps_map));
    }

    // Provides
    if let Some(provides) = &manifest.provides {
        let mut provides_obj = HashMap::new();
        if let Some(nodetypes) = &provides.nodetypes {
            provides_obj.insert(
                "nodetypes".to_string(),
                PropertyValue::Array(
                    nodetypes
                        .iter()
                        .map(|nt| PropertyValue::String(nt.clone()))
                        .collect(),
                ),
            );
        }
        if let Some(workspaces) = &provides.workspaces {
            provides_obj.insert(
                "workspaces".to_string(),
                PropertyValue::Array(
                    workspaces
                        .iter()
                        .map(|ws| PropertyValue::String(ws.clone()))
                        .collect(),
                ),
            );
        }
        if let Some(content) = &provides.content {
            provides_obj.insert(
                "content".to_string(),
                PropertyValue::Array(
                    content
                        .iter()
                        .map(|c| PropertyValue::String(c.clone()))
                        .collect(),
                ),
            );
        }
        properties.insert("provides".to_string(), PropertyValue::Object(provides_obj));
    }

    // Workspace patches
    if let Some(workspace_patches) = &manifest.workspace_patches {
        let mut patches_obj = HashMap::new();
        for (ws_name, patch) in workspace_patches {
            let mut patch_map = HashMap::new();
            if let Some(allowed_node_types) = &patch.allowed_node_types {
                let mut ant_map = HashMap::new();
                if let Some(add) = &allowed_node_types.add {
                    ant_map.insert(
                        "add".to_string(),
                        PropertyValue::Array(
                            add.iter()
                                .map(|nt| PropertyValue::String(nt.clone()))
                                .collect(),
                        ),
                    );
                }
                patch_map.insert(
                    "allowed_node_types".to_string(),
                    PropertyValue::Object(ant_map),
                );
            }
            patches_obj.insert(ws_name.clone(), PropertyValue::Object(patch_map));
        }
        properties.insert(
            "workspace_patches".to_string(),
            PropertyValue::Object(patches_obj),
        );
    }

    properties
}
