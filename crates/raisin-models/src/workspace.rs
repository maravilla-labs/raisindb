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

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::nodes::types::initial_structure::InitialNodeStructure;
use crate::timestamp::StorageTimestamp;

pub mod delta;
pub use delta::DeltaOp;

/// Workspace configuration for branch and NodeType version management
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct WorkspaceConfig {
    /// Default branch for this workspace
    #[serde(default = "default_branch_name")]
    pub default_branch: String,

    /// NodeType revision pinning: maps NodeType name to specific revision (HLC)
    /// None means "track latest", Some(hlc) means "pin to this HLC revision"
    #[serde(default, rename = "node_type_pins", alias = "node_type_refs")]
    pub node_type_pins: HashMap<String, Option<raisin_hlc::HLC>>,
}

fn default_branch_name() -> String {
    "main".to_string()
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            default_branch: default_branch_name(),
            node_type_pins: HashMap::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Workspace {
    pub name: String, // Name of the workspace
    #[serde(default)]
    pub description: Option<String>, // Description of the workspace
    pub allowed_node_types: Vec<String>, // NodeTypes that are allowed in this workspace (namespace:node_type)
    pub allowed_root_node_types: Vec<String>, // NodeTypes that can be root-level types in this workspace (namespace:node_type)
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub initial_structure: Option<InitialNodeStructure>, // Initial root-level nodes to create when workspace is created
    #[serde(default = "default_created_at")]
    pub created_at: StorageTimestamp, // Timestamp for when the workspace was created (i64 nanos in binary, RFC3339 in JSON)
    #[serde(default)]
    pub updated_at: Option<StorageTimestamp>, // Timestamp for when the workspace was last updated (i64 nanos in binary, RFC3339 in JSON)
    #[serde(default)]
    pub config: WorkspaceConfig, // Workspace configuration
}
fn default_created_at() -> StorageTimestamp {
    StorageTimestamp::now()
}

impl Workspace {
    pub fn new(name: String) -> Self {
        Workspace {
            name,
            allowed_node_types: Vec::new(),
            allowed_root_node_types: Vec::new(),
            depends_on: Vec::new(),
            initial_structure: None,
            created_at: StorageTimestamp::now(),
            description: None,
            updated_at: None,
            config: WorkspaceConfig::default(),
        }
    }

    pub fn update_allowed_node_types(
        &mut self,
        allowed_node_types: Vec<String>,
        allowed_root_node_types: Vec<String>,
    ) {
        self.allowed_node_types = allowed_node_types;
        self.allowed_root_node_types = allowed_root_node_types;
        self.updated_at = Some(StorageTimestamp::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    #[test]
    fn deserializes_workspace_from_map_initial_structure() {
        let value = json!({
            "name": "access_control",
            "allowed_node_types": ["raisin:User", "raisin:Role"],
            "allowed_root_node_types": ["raisin:User", "raisin:Role"],
            "depends_on": [],
            "initial_structure": {
                "children": [
                    {"name": "Users", "node_type": "raisin:AclFolder"},
                    {"name": "Roles", "node_type": "raisin:AclFolder"}
                ]
            },
            "config": {
                "default_branch": "main",
                "node_type_pins": {}
            }
        });

        let workspace: Workspace =
            serde_json::from_value(value).expect("map-based workspace should deserialize");

        let children = workspace
            .initial_structure
            .as_ref()
            .and_then(|s| s.children.as_ref())
            .expect("children must be present");

        assert_eq!(children.len(), 2);
        assert_eq!(children[0].name, "Users");
        assert_eq!(children[1].name, "Roles");
    }

    #[test]
    fn deserializes_workspace_from_map_with_rfc3339_timestamps() {
        let value = json!({
            "name": "test_workspace",
            "allowed_node_types": ["raisin:User"],
            "allowed_root_node_types": ["raisin:User"],
            "depends_on": [],
            "created_at": "2023-11-14T22:13:20Z",
            "updated_at": "2023-11-14T22:21:40Z"
        });

        let workspace: Workspace =
            serde_json::from_value(value).expect("RFC3339 timestamps should deserialize");

        assert_eq!(workspace.name, "test_workspace");
        assert_eq!(workspace.created_at.timestamp(), 1_700_000_000);
        assert_eq!(
            workspace
                .updated_at
                .expect("expected updated_at")
                .timestamp(),
            1_700_000_500
        );
    }

    #[test]
    fn serializes_workspace_with_rfc3339_timestamps() {
        let workspace = Workspace {
            name: "test_workspace".to_string(),
            description: None,
            allowed_node_types: vec!["raisin:User".to_string()],
            allowed_root_node_types: vec!["raisin:User".to_string()],
            depends_on: vec![],
            initial_structure: None,
            created_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap().into(),
            updated_at: Some(Utc.timestamp_opt(1_700_000_500, 0).unwrap().into()),
            config: WorkspaceConfig::default(),
        };

        let json = serde_json::to_value(&workspace).expect("should serialize");

        // Verify timestamps are RFC3339 strings (chrono's to_rfc3339 uses +00:00 format)
        assert!(json["created_at"].as_str().unwrap().contains("2023-11-14"));
        assert!(json["updated_at"].as_str().unwrap().contains("2023-11-14"));
    }
}
