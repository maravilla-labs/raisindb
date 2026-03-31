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

/// A reference to a related node in the graph database.
///
/// Represents a directed relationship from a source node to a target node,
/// potentially across workspace boundaries. Stores both the semantic relationship
/// type and the target node's type for efficient filtering.
///
/// # Fields
///
/// * `target` - The ID of the target node
/// * `workspace` - The workspace containing the target node (enables cross-workspace relationships)
/// * `target_node_type` - The node type of the target (e.g., "raisin:Page", "raisin:Asset")
/// * `relation_type` - Semantic relationship type (e.g., "references", "links_to", "authored_by")
/// * `weight` - Optional weight for graph algorithms (e.g., PageRank, shortest path)
///
/// # CRDT Semantics
///
/// Relations use Last-Write-Wins (LWW) conflict resolution based on the composite key
/// (source_id, target_id, relation_type). Only one relation of a given type can exist
/// between two nodes. When concurrent updates occur, the one with the higher HLC timestamp wins.
///
/// # Examples
///
/// ```
/// use raisin_models::nodes::RelationRef;
///
/// // Create a semantic "references" relationship to a Page
/// let relation = RelationRef::new(
///     "node-123".to_string(),
///     "main".to_string(),
///     "raisin:Page".to_string(),
///     "references".to_string(),
///     Some(1.0)
/// );
/// ```
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RelationRef {
    /// ID of the target node
    pub target: String,

    /// Workspace containing the target node
    pub workspace: String,

    /// Node type of the target (for Cypher label filtering)
    /// Examples: "raisin:Page", "raisin:Asset", "raisin:Folder"
    #[serde(default)]
    pub target_node_type: String,

    /// Semantic relationship type (e.g., "references", "links_to", "FRIENDS_WITH")
    #[serde(default)]
    pub relation_type: String,

    /// Optional weight for graph algorithms
    #[serde(default)]
    pub weight: Option<f32>,
}

impl RelationRef {
    /// Create a new relationship reference
    ///
    /// # Arguments
    ///
    /// * `target` - ID of the target node
    /// * `workspace` - Workspace containing the target
    /// * `target_node_type` - Node type of the target (e.g., "raisin:Page")
    /// * `relation_type` - Semantic relationship type (e.g., "references")
    /// * `weight` - Optional weight for graph algorithms
    pub fn new(
        target: String,
        workspace: String,
        target_node_type: String,
        relation_type: String,
        weight: Option<f32>,
    ) -> Self {
        Self {
            target,
            workspace,
            target_node_type,
            relation_type,
            weight,
        }
    }

    /// Create a relationship without weight
    pub fn simple(
        target: String,
        workspace: String,
        target_node_type: String,
        relation_type: String,
    ) -> Self {
        Self::new(target, workspace, target_node_type, relation_type, None)
    }

    /// Get a unique key for this relationship target
    ///
    /// Returns a string in the format "{workspace}:{target_id}" that uniquely
    /// identifies the target across workspaces.
    pub fn target_key(&self) -> String {
        format!("{}:{}", self.workspace, self.target)
    }

    /// Create a RelationRef from node ID, workspace, and types
    ///
    /// Helper for quickly creating relationships when you have the basic info.
    pub fn from_node_id(
        node_id: impl Into<String>,
        workspace: impl Into<String>,
        node_type: impl Into<String>,
        relation_type: impl Into<String>,
    ) -> Self {
        Self::simple(
            node_id.into(),
            workspace.into(),
            node_type.into(),
            relation_type.into(),
        )
    }
}

/// A complete relationship record with both source and target information.
///
/// This structure is used in the global relationship index to enable efficient
/// cross-workspace graph queries. Unlike `RelationRef` which only stores target
/// information (optimized for per-node queries), `FullRelation` stores complete
/// information about both endpoints including their node types.
///
/// # Use Cases
///
/// - Global relationship scans: `MATCH (a)-[:references]->(b)` across all workspaces
/// - Cross-workspace queries without knowing source node
/// - Cypher pattern matching where relationships are scanned first
/// - Efficient label-based filtering: `MATCH (a:RaisinPage)-[r]->(b:RaisinAsset)`
///
/// # Fields
///
/// * `source_id` - ID of the source node
/// * `source_workspace` - Workspace containing the source node
/// * `source_node_type` - Node type of source (e.g., "raisin:Folder")
/// * `target_id` - ID of the target node
/// * `target_workspace` - Workspace containing the target node
/// * `target_node_type` - Node type of target (e.g., "raisin:Page")
/// * `relation_type` - Semantic relationship type (e.g., "references", "links_to")
/// * `weight` - Optional weight for graph algorithms
///
/// # CRDT Semantics
///
/// Relations use Last-Write-Wins (LWW) conflict resolution based on the composite key
/// (source_id, target_id, relation_type). Only one relation of a given type can exist
/// between two nodes. When concurrent updates occur, the one with the higher HLC timestamp wins.
///
/// # Examples
///
/// ```
/// use raisin_models::nodes::FullRelation;
///
/// // Create a cross-workspace "references" relationship
/// let relation = FullRelation::new(
///     "node-123".to_string(),
///     "content".to_string(),
///     "raisin:Page".to_string(),
///     "node-456".to_string(),
///     "assets".to_string(),
///     "raisin:Asset".to_string(),
///     "references".to_string(),
///     Some(1.0)
/// );
/// ```
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FullRelation {
    /// ID of the source node
    pub source_id: String,

    /// Workspace containing the source node
    pub source_workspace: String,

    /// Node type of the source (for Cypher label filtering)
    /// Examples: "raisin:Page", "raisin:Folder"
    #[serde(default)]
    pub source_node_type: String,

    /// ID of the target node
    pub target_id: String,

    /// Workspace containing the target node
    pub target_workspace: String,

    /// Node type of the target (for Cypher label filtering)
    /// Examples: "raisin:Page", "raisin:Asset"
    #[serde(default)]
    pub target_node_type: String,

    /// Semantic relationship type (e.g., "references", "links_to", "FRIENDS_WITH")
    #[serde(default)]
    pub relation_type: String,

    /// Optional weight for graph algorithms
    #[serde(default)]
    pub weight: Option<f32>,
}

impl FullRelation {
    /// Create a new full relationship record
    ///
    /// # Arguments
    ///
    /// * `source_id` - ID of the source node
    /// * `source_workspace` - Workspace containing the source
    /// * `source_node_type` - Node type of the source
    /// * `target_id` - ID of the target node
    /// * `target_workspace` - Workspace containing the target
    /// * `target_node_type` - Node type of the target
    /// * `relation_type` - Semantic type of the relationship
    /// * `weight` - Optional weight for graph algorithms
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source_id: String,
        source_workspace: String,
        source_node_type: String,
        target_id: String,
        target_workspace: String,
        target_node_type: String,
        relation_type: String,
        weight: Option<f32>,
    ) -> Self {
        Self {
            source_id,
            source_workspace,
            source_node_type,
            target_id,
            target_workspace,
            target_node_type,
            relation_type,
            weight,
        }
    }

    /// Create a FullRelation from source info and a RelationRef
    ///
    /// Helper to convert from the forward index format (source + RelationRef)
    /// to the complete relationship format needed for the global index.
    pub fn from_source_and_ref(
        source_id: String,
        source_workspace: String,
        source_node_type: String,
        relation_ref: &RelationRef,
    ) -> Self {
        Self {
            source_id,
            source_workspace,
            source_node_type,
            target_id: relation_ref.target.clone(),
            target_workspace: relation_ref.workspace.clone(),
            target_node_type: relation_ref.target_node_type.clone(),
            relation_type: relation_ref.relation_type.clone(),
            weight: relation_ref.weight,
        }
    }

    /// Convert to a RelationRef (target information only)
    ///
    /// Useful when interfacing with APIs that expect RelationRef format.
    pub fn to_relation_ref(&self) -> RelationRef {
        RelationRef {
            target: self.target_id.clone(),
            workspace: self.target_workspace.clone(),
            target_node_type: self.target_node_type.clone(),
            relation_type: self.relation_type.clone(),
            weight: self.weight,
        }
    }
}
