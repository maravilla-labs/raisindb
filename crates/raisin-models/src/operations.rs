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

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Types of operations that can be performed on nodes
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OperationType {
    /// Node was moved to a new parent or path
    Move {
        /// Original path before move
        from_path: String,
        /// Original parent ID before move
        from_parent_id: String,
        /// New path after move
        to_path: String,
        /// New parent ID after move
        to_parent_id: String,
    },
    /// Node was copied from another node
    Copy {
        /// Source node ID that was copied
        source_id: String,
        /// Source node path at time of copy
        source_path: String,
        /// Destination path where copy was created
        destination_path: String,
    },
    /// Node was renamed
    Rename {
        /// Original name before rename
        old_name: String,
        /// New name after rename
        new_name: String,
    },
    /// Node was reordered within its parent
    Reorder {
        /// Original fractional index
        old_index: String,
        /// New fractional index
        new_index: String,
    },
}

/// Metadata about a node operation performed in a revision
///
/// This structure captures the details of what operation was performed
/// on a node, similar to how TranslationMeta captures translation operations.
/// This enables time-travel queries and audit trails.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationMeta {
    /// The type of operation and its specific details
    pub operation: OperationType,

    /// Revision number where this operation occurred
    pub revision: raisin_hlc::HLC,

    /// Parent revision (for tracking operation history)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_revision: Option<raisin_hlc::HLC>,

    /// When the operation was performed
    pub timestamp: DateTime<Utc>,

    /// Who performed the operation (user ID, system, etc.)
    pub actor: String,

    /// Descriptive message about why the operation was performed
    pub message: String,

    /// Whether this was a system-initiated operation
    #[serde(default)]
    pub is_system: bool,

    /// Node ID that this operation was performed on
    pub node_id: String,
}

impl OperationMeta {
    /// Create a new OperationMeta for a move operation
    #[allow(clippy::too_many_arguments)]
    pub fn new_move(
        node_id: String,
        from_path: String,
        from_parent_id: String,
        to_path: String,
        to_parent_id: String,
        revision: &raisin_hlc::HLC,
        parent_revision: Option<&raisin_hlc::HLC>,
        actor: String,
        message: String,
    ) -> Self {
        Self {
            operation: OperationType::Move {
                from_path,
                from_parent_id,
                to_path,
                to_parent_id,
            },
            revision: *revision,
            parent_revision: parent_revision.copied(),
            timestamp: Utc::now(),
            actor,
            message,
            is_system: false,
            node_id,
        }
    }

    /// Create a new OperationMeta for a copy operation
    #[allow(clippy::too_many_arguments)]
    pub fn new_copy(
        node_id: String,
        source_id: String,
        source_path: String,
        destination_path: String,
        revision: &raisin_hlc::HLC,
        parent_revision: Option<&raisin_hlc::HLC>,
        actor: String,
        message: String,
    ) -> Self {
        Self {
            operation: OperationType::Copy {
                source_id,
                source_path,
                destination_path,
            },
            revision: *revision,
            parent_revision: parent_revision.copied(),
            timestamp: Utc::now(),
            actor,
            message,
            is_system: false,
            node_id,
        }
    }

    /// Create a new OperationMeta for a rename operation
    pub fn new_rename(
        node_id: String,
        old_name: String,
        new_name: String,
        revision: &raisin_hlc::HLC,
        parent_revision: Option<&raisin_hlc::HLC>,
        actor: String,
        message: String,
    ) -> Self {
        Self {
            operation: OperationType::Rename { old_name, new_name },
            revision: *revision,
            parent_revision: parent_revision.copied(),
            timestamp: Utc::now(),
            actor,
            message,
            is_system: false,
            node_id,
        }
    }

    /// Create a new OperationMeta for a reorder operation
    pub fn new_reorder(
        node_id: String,
        old_index: String,
        new_index: String,
        revision: &raisin_hlc::HLC,
        parent_revision: Option<&raisin_hlc::HLC>,
        actor: String,
        message: String,
    ) -> Self {
        Self {
            operation: OperationType::Reorder {
                old_index,
                new_index,
            },
            revision: *revision,
            parent_revision: parent_revision.copied(),
            timestamp: Utc::now(),
            actor,
            message,
            is_system: false,
            node_id,
        }
    }
}
