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

//! Deep node and children expansion types for API responses.

use serde::{Deserialize, Serialize};

use super::definition::Node;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeepNode {
    pub node: Node,
    pub children: std::collections::HashMap<String, DeepNode>,
}

impl DeepNode {
    pub fn new(node: Node) -> Self {
        Self {
            node,
            children: Default::default(),
        }
    }
}

/// Children field that can be either string names or expanded nodes
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChildrenField {
    /// Just the names when we haven't expanded to this depth
    Names(Vec<String>),
    /// Full nodes when we've expanded to this depth
    Nodes(Vec<Box<NodeWithChildren>>),
}

/// Minimal wrapper that changes just the children field for API responses.
/// Uses serde flatten to include all Node fields without duplication.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeWithChildren {
    /// Flatten includes all fields from Node except children
    #[serde(flatten)]
    pub node: Node,
    /// Override the children field with our flexible enum
    pub children: ChildrenField,
}

impl NodeWithChildren {
    pub fn new(mut node: Node) -> Self {
        // Extract the children to use in our enum
        let children_names = std::mem::take(&mut node.children);
        Self {
            node,
            children: ChildrenField::Names(children_names),
        }
    }

    pub fn with_children(mut self, children: Vec<NodeWithChildren>) -> Self {
        self.children = ChildrenField::Nodes(children.into_iter().map(Box::new).collect());
        self
    }

    pub fn with_string_children(mut self, children: Vec<String>) -> Self {
        self.children = ChildrenField::Names(children);
        self
    }
}
