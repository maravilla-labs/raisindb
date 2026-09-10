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

//! Package manifest types

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Package manifest structure
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
    /// Whether this builtin package is auto-installed into every repo on boot.
    /// Defaults to `true` when absent; parsed here so the field round-trips.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_install: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<Vec<PackageDependency>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provides: Option<PackageProvides>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_patches: Option<HashMap<String, WorkspacePatch>>,
    /// Informational list of locales provided by this package.
    /// The actual translation files are the source of truth.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locales: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageMigration {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<MigrationOperation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationOperation {
    ReplaceNodeType(ReplaceNodeTypeMigration),
    PatchNodes(PatchNodesMigration),
    MoveNode(MoveNodeMigration),
    DeleteNode(DeleteNodeMigration),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplaceNodeTypeMigration {
    pub workspace: String,
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub archetype_from: Option<String>,
    #[serde(default)]
    pub archetype_to: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchNodesMigration {
    pub workspace: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub node_type: Option<String>,
    #[serde(default)]
    pub properties: HashMap<String, raisin_models::nodes::properties::value::PropertyValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveNodeMigration {
    pub workspace: String,
    pub from: String,
    pub to: String,
    #[serde(default = "default_collision")]
    pub on_collision: CollisionMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteNodeMigration {
    pub workspace: String,
    pub path: String,
    #[serde(default = "default_true")]
    pub if_empty: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CollisionMode {
    Fail,
    Skip,
}

fn default_collision() -> CollisionMode {
    CollisionMode::Fail
}

fn default_true() -> bool {
    true
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
    /// Default node type for auto-created folders in this workspace
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_folder_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllowedNodeTypesPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub add: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auto_install_field_round_trips() {
        // Absent → None (treated as auto-install by the boot scan default).
        let bare: PackageManifest = serde_yaml::from_str("name: x\nversion: 1.0.0\n").unwrap();
        assert_eq!(bare.auto_install, None);

        // Explicit opt-out parses as Some(false).
        let opted_out: PackageManifest =
            serde_yaml::from_str("name: x\nversion: 1.0.0\nauto_install: false\n").unwrap();
        assert_eq!(opted_out.auto_install, Some(false));
    }
}
