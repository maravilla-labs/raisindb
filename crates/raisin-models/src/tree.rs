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

//! Content-addressed tree structures for immutable revision snapshots
//!
//! Trees provide Git-like structural sharing for efficient revision storage.
//! Each commit creates a new root tree ID that references only changed pages.

use base64::Engine;
use raisin_hlc::HLC;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Type of change operation in a revision
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ChangeOperation {
    /// Node was added in this revision
    Added,
    /// Node was modified in this revision
    Modified,
    /// Node was deleted in this revision
    Deleted,
    /// Node was reordered among siblings in this revision
    Reordered,
}

/// Details of a node change in a revision
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeChange {
    /// ID of the node that changed
    pub node_id: String,

    /// Type of change operation
    pub operation: ChangeOperation,

    /// Node type (for quick filtering/display)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_type: Option<String>,

    /// Path of the node (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,

    /// Translation locale if this change was to a translation
    /// - None: Base node was changed
    /// - Some("fr"): French translation was changed
    /// - Some("de-CH"): Swiss German translation was changed
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub translation_locale: Option<String>,
}

impl NodeChange {
    /// Create a new node change record
    pub fn new(node_id: String, operation: ChangeOperation) -> Self {
        Self {
            node_id,
            operation,
            node_type: None,
            path: None,
            translation_locale: None,
        }
    }

    /// Add node type to the change record
    pub fn with_node_type(mut self, node_type: String) -> Self {
        self.node_type = Some(node_type);
        self
    }

    /// Add path to the change record
    pub fn with_path(mut self, path: String) -> Self {
        self.path = Some(path);
        self
    }

    /// Add translation locale to the change record
    /// Use this to mark that a translation was changed, not the base node
    pub fn with_translation_locale(mut self, locale: String) -> Self {
        self.translation_locale = Some(locale);
        self
    }

    /// Check if this change is to a translation (vs base node)
    pub fn is_translation_change(&self) -> bool {
        self.translation_locale.is_some()
    }

    /// Get the translation locale if this is a translation change
    pub fn get_translation_locale(&self) -> Option<&str> {
        self.translation_locale.as_deref()
    }
}

/// Entry in an immutable tree page (represents one node in a directory listing)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreeEntry {
    /// Sort key (node name for children, node_id for roots)
    /// Must be stable and sortable for consistent tree hashing
    pub entry_key: String,

    /// Node ID this entry points to
    pub node_id: String,

    /// If this node has children, points to their tree page ID
    /// None = leaf node or empty directory
    pub children_tree_id: Option<[u8; 32]>,

    /// Node type (for filtering without resolving snapshot)
    /// Denormalized for performance: avoid snapshot lookup for type queries
    pub node_type: String,
}

impl TreeEntry {
    /// Create a new tree entry for a node
    pub fn new(entry_key: String, node_id: String, node_type: String) -> Self {
        Self {
            entry_key,
            node_id,
            children_tree_id: None,
            node_type,
        }
    }

    /// Create entry with children tree
    pub fn with_children(mut self, children_tree_id: [u8; 32]) -> Self {
        self.children_tree_id = Some(children_tree_id);
        self
    }
}

/// Commit metadata with root tree ID (extends existing RevisionMeta)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeCommitMeta {
    /// Revision number
    pub revision: u64,

    /// Parent revision (for history walking)
    pub parent_rev: Option<u64>,

    /// Root tree ID (content hash of root directory listing)
    /// This is the entry point for reading the entire repository state at this revision
    pub root_tree_id: [u8; 32],

    /// Commit message
    pub message: String,

    /// Actor who made the commit
    pub actor: String,

    /// Timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,

    /// Branch name
    pub branch: String,

    /// Number of nodes changed in this commit
    /// Optional for backward compatibility with old revisions
    #[serde(default)]
    pub nodes_changed: usize,

    /// Whether this is a system commit (background job, auto-merge, etc.)
    #[serde(default)]
    pub is_system: bool,

    /// Whether this revision is a manual version created by the user
    /// Manual versions are user-created snapshots for specific nodes
    #[serde(default)]
    pub is_manual_version: bool,

    /// Node ID this manual version applies to (if is_manual_version is true)
    /// This allows filtering revision history to show only manual versions for a specific node
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_version_node_id: Option<String>,
}

/// Workspace delta operation for live editing (before commit)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DeltaOp {
    /// Add new node
    Add {
        /// Full node data serialized
        node_data: Vec<u8>,
    },

    /// Update existing node
    Update {
        /// Full node data serialized
        node_data: Vec<u8>,
    },

    /// Delete node
    Delete,
}

/// Workspace delta record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceDelta {
    /// Operation type
    pub op: DeltaOp,

    /// When this delta was created
    pub timestamp: chrono::DateTime<chrono::Utc>,

    /// Actor who made the change
    pub actor: Option<String>,
}

/// Workspace configuration mode
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum WorkspaceMode {
    /// Full revision tracking with immutable trees
    /// Use for: Editorial content, versioned APIs, audited workflows
    Versioned {
        /// Base revision this workspace is editing from
        base_revision: u64,

        /// Whether to auto-commit on every change
        /// - true: Every node operation creates a new revision (expensive but simple)
        /// - false: Changes accumulate in delta log until explicit commit
        auto_commit: bool,
    },

    /// Live-edit mode with delta log only (no immutable trees)
    /// Use for: User profiles, sessions, real-time collaboration, chat
    /// Commits are cheap (just flush delta log) but no time-travel
    Live {
        /// Whether to keep a delta history (for undo/redo)
        keep_deltas: bool,

        /// Max delta history size (0 = no history, just current state)
        max_deltas: usize,
    },

    /// Ephemeral workspace (deleted on close)
    /// Use for: Temporary previews, sandbox testing, branches that auto-delete
    Ephemeral,
}

impl Default for WorkspaceMode {
    fn default() -> Self {
        WorkspaceMode::Versioned {
            base_revision: 0,
            auto_commit: false,
        }
    }
}

/// Extended workspace configuration for tree-based revision system
///
/// This extends the basic WorkspaceConfig from workspace.rs with tree-specific settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeWorkspaceConfig {
    /// Workspace identifier
    pub workspace_id: String,

    /// Workspace mode (Versioned, Live, or Ephemeral)
    pub mode: WorkspaceMode,

    /// Default branch for this workspace
    pub default_branch: String,

    /// NodeType version pinning
    /// - Key: NodeType name (e.g., "raisin:Page")
    /// - Value: None = track latest, Some(rev) = pinned to specific revision
    #[serde(default, rename = "node_type_pins", alias = "node_type_refs")]
    pub node_type_pins: HashMap<String, Option<u64>>,
}

impl Default for TreeWorkspaceConfig {
    fn default() -> Self {
        Self {
            workspace_id: String::new(),
            mode: WorkspaceMode::default(),
            default_branch: "main".to_string(),
            node_type_pins: HashMap::new(),
        }
    }
}

/// Cursor for pagination (MongoDB-style)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageCursor {
    /// Last entry key from previous page
    /// Next query starts AFTER this key
    pub last_key: String,

    /// Lock pagination to specific revision (for consistency)
    /// If None, uses current HEAD (may shift between pages)
    pub revision: Option<HLC>,

    /// Opaque token to prevent tampering
    /// Hash of (last_key, revision, secret)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

impl PageCursor {
    /// Create cursor from last entry key
    pub fn new(last_key: String, revision: Option<HLC>) -> Self {
        Self {
            last_key,
            revision,
            token: None,
        }
    }

    /// Encode cursor as base64 string for API responses
    pub fn encode(&self) -> Result<String, serde_json::Error> {
        let json = serde_json::to_string(self)?;
        Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json))
    }

    /// Decode cursor from base64 string
    pub fn decode(s: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let json = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s)?;
        let cursor = serde_json::from_slice(&json)?;
        Ok(cursor)
    }
}

/// Paginated response with cursor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page<T> {
    /// Items in this page
    pub items: Vec<T>,

    /// Cursor for next page (None = last page)
    pub next_cursor: Option<PageCursor>,

    /// Total count (if available/requested)
    pub total: Option<usize>,
}

impl<T> Page<T> {
    /// Create a page with items and optional next cursor
    pub fn new(items: Vec<T>, next_cursor: Option<PageCursor>) -> Self {
        Self {
            items,
            next_cursor,
            total: None,
        }
    }

    /// Add total count
    pub fn with_total(mut self, total: usize) -> Self {
        self.total = Some(total);
        self
    }
}
