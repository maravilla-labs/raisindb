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

//! Which data a server's AUTO-GENERATED tools may touch.
//!
//! One tool is generated per enabled [`DataOperation`], scoped to the declared
//! workspaces. Row-level security still applies on top of everything here — a
//! data policy widens what the server offers, never what a caller may see.

use serde::{Deserialize, Serialize};

/// A data operation an MCP server may auto-expose as a tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataOperation {
    /// `query_nodes` — list/filter nodes by type within a workspace.
    QueryNodes,
    /// `get_node` — fetch a single node by path.
    GetNode,
    /// `search_nodes` — full-text / vector search.
    SearchNodes,
    /// `create_node` — create a new node.
    CreateNode,
    /// `update_node` — update an existing node.
    UpdateNode,
    /// `delete_node` — delete a node.
    DeleteNode,
    /// `move_node` — reparent a node (and its subtree).
    MoveNode,
    /// `reorder_node` — reposition a node among its siblings (editorial order).
    ReorderNode,
    /// `list_children` — a parent's direct children, in editorial order.
    ListChildren,
    /// `list_workspaces` — list the workspaces this server exposes.
    ListWorkspaces,
}

impl DataOperation {
    /// The full set of operations, in stable order.
    pub const ALL: [DataOperation; 10] = [
        DataOperation::QueryNodes,
        DataOperation::GetNode,
        DataOperation::SearchNodes,
        DataOperation::ListChildren,
        DataOperation::CreateNode,
        DataOperation::UpdateNode,
        DataOperation::DeleteNode,
        DataOperation::MoveNode,
        DataOperation::ReorderNode,
        DataOperation::ListWorkspaces,
    ];

    /// Whether this operation mutates content.
    pub fn is_write(self) -> bool {
        matches!(
            self,
            DataOperation::CreateNode
                | DataOperation::UpdateNode
                | DataOperation::DeleteNode
                | DataOperation::MoveNode
                | DataOperation::ReorderNode
        )
    }

    /// Parse an operation from its wire name.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "query_nodes" => Some(Self::QueryNodes),
            "get_node" => Some(Self::GetNode),
            "search_nodes" => Some(Self::SearchNodes),
            "create_node" => Some(Self::CreateNode),
            "update_node" => Some(Self::UpdateNode),
            "delete_node" => Some(Self::DeleteNode),
            "move_node" => Some(Self::MoveNode),
            "reorder_node" => Some(Self::ReorderNode),
            "list_children" => Some(Self::ListChildren),
            "list_workspaces" => Some(Self::ListWorkspaces),
            _ => None,
        }
    }
}

/// Which data the auto-generated tools of a server may touch.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DataPolicy {
    /// Workspaces the server may expose. Empty = none.
    #[serde(default)]
    pub workspaces: Vec<String>,
    /// Operations the server enables. Empty = none.
    #[serde(default)]
    pub operations: Vec<DataOperation>,
    /// Whether `raisin://` resources are exposed for the workspaces above.
    #[serde(default)]
    pub resources: bool,
}

impl DataPolicy {
    /// Whether `op` is enabled by this policy.
    pub fn allows(&self, op: DataOperation) -> bool {
        self.operations.contains(&op)
    }

    /// Whether `workspace` is in scope for this policy.
    pub fn covers_workspace(&self, workspace: &str) -> bool {
        self.workspaces.iter().any(|w| w == workspace)
    }
}
