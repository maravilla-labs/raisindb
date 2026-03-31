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

//! Shared types for repository management: change info, revision metadata, and GC stats

use raisin_hlc::HLC;
use raisin_models::operations::OperationMeta;
use raisin_models::tree::ChangeOperation;
use serde::{Deserialize, Serialize};

/// Statistics from a garbage collection run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GarbageCollectionStats {
    /// Number of revisions examined
    pub revisions_examined: usize,

    /// Number of revisions marked as reachable
    pub revisions_reachable: usize,

    /// Number of revisions deleted
    pub revisions_deleted: usize,

    /// Number of node snapshots deleted
    pub snapshots_deleted: usize,

    /// Bytes reclaimed (approximate)
    pub bytes_reclaimed: u64,

    /// Duration of GC run in milliseconds
    pub duration_ms: u64,
}

/// Information about a node change in a revision
///
/// Tracks which node changed, in which workspace, and what type of change occurred.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeChangeInfo {
    /// ID of the node that changed
    pub node_id: String,

    /// Workspace where the change occurred
    pub workspace: String,

    /// Type of change operation (Added, Modified, Deleted)
    pub operation: ChangeOperation,

    /// Translation locale if this change was to a translation
    /// - None: Base node was changed
    /// - Some("fr"): French translation was changed
    /// - Some("de-CH"): Swiss German translation was changed
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub translation_locale: Option<String>,
}

/// Information about a NodeType change in a revision
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeTypeChangeInfo {
    /// Name of the NodeType that changed
    pub name: String,
    /// Operation performed on the NodeType (Added, Modified, Deleted)
    pub operation: ChangeOperation,
}

/// Information about an Archetype change in a revision
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArchetypeChangeInfo {
    /// Name of the Archetype that changed
    pub name: String,
    /// Operation performed on the Archetype (Added, Modified, Deleted)
    pub operation: ChangeOperation,
}

/// Information about an ElementType change in a revision
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ElementTypeChangeInfo {
    /// Name of the ElementType that changed
    pub name: String,
    /// Operation performed on the ElementType (Added, Modified, Deleted)
    pub operation: ChangeOperation,
}

/// Revision metadata for Git-like commits
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RevisionMeta {
    /// Revision (Hybrid Logical Clock)
    pub revision: HLC,

    /// Parent revision (None for initial commit)
    pub parent: Option<HLC>,

    /// Second parent for merge commits (None for regular commits)
    /// When present, this revision represents a merge between `parent` and `merge_parent`
    #[serde(default)]
    pub merge_parent: Option<HLC>,

    /// Branch this commit was made on
    pub branch: String,

    /// Timestamp of commit
    pub timestamp: chrono::DateTime<chrono::Utc>,

    /// Actor who made the commit (user ID, system, etc.)
    pub actor: String,

    /// Commit message
    pub message: String,

    /// Whether this is a system commit (background job, etc.)
    pub is_system: bool,

    /// List of node changes in this revision with workspace and operation information
    #[serde(default)]
    pub changed_nodes: Vec<NodeChangeInfo>,

    /// List of NodeType changes in this revision with operation information
    #[serde(default)]
    pub changed_node_types: Vec<NodeTypeChangeInfo>,

    /// List of Archetype changes in this revision with operation information
    #[serde(default)]
    pub changed_archetypes: Vec<ArchetypeChangeInfo>,

    /// List of ElementType changes in this revision with operation information
    #[serde(default)]
    pub changed_element_types: Vec<ElementTypeChangeInfo>,

    /// Optional operation metadata describing what operation was performed
    /// (move, copy, rename, reorder) for audit trails and time-travel queries
    pub operation: Option<OperationMeta>,
}
