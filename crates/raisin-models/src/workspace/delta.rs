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

use crate::nodes::Node;
use serde::{Deserialize, Serialize};

/// Delta operation types for workspace changes
///
/// This enum represents the operations that can be performed in a workspace
/// before committing. Using an enum (instead of just storing nodes) allows
/// us to properly track deletions via tombstones.
///
/// # Tombstone Pattern
///
/// When a node is deleted, we don't remove it from the workspace delta.
/// Instead, we create a `Delete` tombstone. This ensures that on commit:
/// - We know which nodes to exclude from the new tree
/// - Deleted nodes don't "resurrect" from the parent tree
/// - Queries see the tombstone and return None (node appears deleted)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum DeltaOp {
    /// Create or update a node
    Upsert(Box<Node>),

    /// Delete a node (tombstone)
    /// Stores node_id and path so we know what to exclude from the next tree
    Delete { node_id: String, path: String },
}

impl DeltaOp {
    /// Get the node ID from this operation
    pub fn node_id(&self) -> &str {
        match self {
            DeltaOp::Upsert(node) => &node.id,
            DeltaOp::Delete { node_id, .. } => node_id,
        }
    }

    /// Get the node path if this is an upsert
    pub fn node_path(&self) -> Option<&str> {
        match self {
            DeltaOp::Upsert(node) => Some(&node.path),
            DeltaOp::Delete { path, .. } => Some(path),
        }
    }

    /// Check if this is a deletion tombstone
    pub fn is_delete(&self) -> bool {
        matches!(self, DeltaOp::Delete { .. })
    }

    /// Check if this is an upsert operation
    pub fn is_upsert(&self) -> bool {
        matches!(self, DeltaOp::Upsert(_))
    }

    /// Get the node if this is an upsert
    pub fn as_node(&self) -> Option<&Node> {
        match self {
            DeltaOp::Upsert(node) => Some(node),
            DeltaOp::Delete { .. } => None,
        }
    }

    /// Convert into node if this is an upsert
    pub fn into_node(self) -> Option<Node> {
        match self {
            DeltaOp::Upsert(node) => Some(*node),
            DeltaOp::Delete { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_delta_op_upsert() {
        let node = Node {
            id: "test-1".into(),
            name: "test".into(),
            path: "/test".into(),
            node_type: "Test".into(),
            properties: HashMap::new(),
            ..Default::default()
        };

        let op = DeltaOp::Upsert(Box::new(node.clone()));

        assert!(op.is_upsert());
        assert!(!op.is_delete());
        assert_eq!(op.node_id(), "test-1");
        assert_eq!(op.node_path(), Some("/test"));
        assert!(op.as_node().is_some());
    }

    #[test]
    fn test_delta_op_delete() {
        let op = DeltaOp::Delete {
            node_id: "test-1".into(),
            path: "/test".into(),
        };

        assert!(op.is_delete());
        assert!(!op.is_upsert());
        assert_eq!(op.node_id(), "test-1");
        assert_eq!(op.node_path(), Some("/test"));
        assert!(op.as_node().is_none());
    }

    #[test]
    fn test_delta_op_serialization() {
        let node = Node {
            id: "test-1".into(),
            name: "test".into(),
            path: "/test".into(),
            node_type: "Test".into(),
            properties: HashMap::new(),
            ..Default::default()
        };

        let upsert = DeltaOp::Upsert(Box::new(node));
        let json = serde_json::to_string(&upsert).unwrap();
        let deserialized: DeltaOp = serde_json::from_str(&json).unwrap();
        assert!(deserialized.is_upsert());

        let delete = DeltaOp::Delete {
            node_id: "test-1".into(),
            path: "/test".into(),
        };
        let json = serde_json::to_string(&delete).unwrap();
        let deserialized: DeltaOp = serde_json::from_str(&json).unwrap();
        assert!(deserialized.is_delete());
    }
}
