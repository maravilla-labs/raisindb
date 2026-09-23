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
///
/// Serialized as the node's own fields with `children` REPLACED by
/// [`ChildrenField`]. A derived `#[serde(flatten)]` cannot do that: `Node` has
/// a `children` field of its own, so the derive wrote the key twice — once
/// from the node (always `[]`, the names having been moved out) and once from
/// the wrapper. A client whose parser keeps the last key saw the right value;
/// a strict one (serde_json among them) refused the document with
/// "duplicate field `children`".
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct NodeWithChildren {
    /// All fields of the node; its own `children` is superseded on output.
    #[serde(flatten)]
    pub node: Node,
    /// Override the children field with our flexible enum
    pub children: ChildrenField,
}

impl Serialize for NodeWithChildren {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::Error;

        let mut value = serde_json::to_value(&self.node).map_err(S::Error::custom)?;
        let children = serde_json::to_value(&self.children).map_err(S::Error::custom)?;
        match value.as_object_mut() {
            Some(object) => {
                object.insert("children".to_string(), children);
            }
            None => return Err(S::Error::custom("a node must serialize as an object")),
        }
        value.serialize(serializer)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str) -> Node {
        Node {
            id: format!("id-{name}"),
            name: name.to_string(),
            path: format!("/{name}"),
            node_type: "t".to_string(),
            children: vec!["kid".to_string()],
            ..Default::default()
        }
    }

    #[test]
    fn children_is_written_once_and_round_trips() {
        let expanded = NodeWithChildren::new(node("parent"))
            .with_children(vec![NodeWithChildren::new(node("kid"))]);

        let text = serde_json::to_string(&expanded).unwrap();
        assert_eq!(
            text.matches("\"children\"").count(),
            // the parent's key, and the one inside the expanded child
            2,
            "{text}"
        );

        let back: NodeWithChildren = serde_json::from_str(&text).unwrap();
        assert_eq!(back.node.name, "parent");
        match back.children {
            ChildrenField::Nodes(kids) => {
                assert_eq!(kids.len(), 1);
                assert_eq!(kids[0].node.name, "kid");
                assert_eq!(kids[0].children, ChildrenField::Names(vec!["kid".into()]));
            }
            other => panic!("expected expanded children, got {other:?}"),
        }
    }
}
