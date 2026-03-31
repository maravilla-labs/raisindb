// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! FlowDefinition struct and implementation
//!
//! Provides the main flow definition type with:
//! - Node indexing for O(1) lookups
//! - Multi-format parsing (compiled, runtime, designer)
//! - Edge generation from next_node fields

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

use super::node_types::{FlowEdge, FlowMetadata, FlowNode, StepType};
use crate::types::designer_format::DesignerFlowDefinition;
use crate::types::FlowError;

/// Complete flow definition parsed from workflow_data
///
/// This represents the workflow structure defined in the visual flow designer.
/// Supports both the runtime format (with step_type and edges) and the designer
/// format (with node_type and children).
///
/// # Performance
///
/// The definition maintains an internal HashMap index for O(1) node lookups.
/// This is critical for workflows with 200+ steps where linear search would be O(n^2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowDefinition {
    /// All nodes in the flow
    pub nodes: Vec<FlowNode>,

    /// Edges connecting nodes (optional - can be generated from next_node fields)
    #[serde(default)]
    pub edges: Vec<FlowEdge>,

    /// Flow-level metadata
    #[serde(default)]
    pub metadata: FlowMetadata,

    /// Node index for O(1) lookups by ID
    /// Built lazily on first access or via `build_index()`
    #[serde(skip)]
    pub(crate) node_index: Option<HashMap<String, usize>>,
}

impl FlowDefinition {
    /// Build the node index for O(1) lookups
    ///
    /// This should be called after constructing or modifying the definition.
    /// Called automatically by `from_workflow_data()`.
    pub fn build_index(&mut self) {
        let mut index = HashMap::with_capacity(self.nodes.len());
        for (i, node) in self.nodes.iter().enumerate() {
            index.insert(node.id.clone(), i);
        }
        // Also index children recursively
        index_children(&mut index, &self.nodes.clone());
        self.node_index = Some(index);
    }

    /// Find a node by ID (O(1) with index, O(n) fallback)
    ///
    /// Uses the internal HashMap index for fast lookups.
    /// Falls back to linear search if index is not built.
    pub fn find_node(&self, node_id: &str) -> Option<&FlowNode> {
        // Try indexed lookup first
        if let Some(ref index) = self.node_index {
            if let Some(&idx) = index.get(node_id) {
                if idx != usize::MAX {
                    return self.nodes.get(idx);
                }
                // Child node - search in children
                return find_in_children(node_id, &self.nodes);
            }
            return None;
        }
        // Fallback to linear search (for backwards compatibility)
        self.nodes
            .iter()
            .find(|n| n.id == node_id)
            .or_else(|| find_in_children(node_id, &self.nodes))
    }

    /// Get the start node
    pub fn start_node(&self) -> Option<&FlowNode> {
        self.nodes
            .iter()
            .find(|n| matches!(n.step_type, StepType::Start))
    }

    /// Get outgoing edges from a node
    pub fn outgoing_edges(&self, node_id: &str) -> Vec<&FlowEdge> {
        self.edges.iter().filter(|e| e.from == node_id).collect()
    }

    /// Get next node ID from a node (for simple sequential flows)
    pub fn next_node_id(&self, node_id: &str) -> Option<String> {
        self.outgoing_edges(node_id)
            .first()
            .map(|edge| edge.to.clone())
    }

    /// Parse workflow_data from any supported format.
    ///
    /// Supports three formats:
    /// 1. **Compiled format** - Has "format": "compiled" field, used directly
    /// 2. **Runtime format** - Has `step_type` on nodes, edges are optional
    /// 3. **Designer format** - Has `node_type` on nodes, tree-based structure
    pub fn from_workflow_data(value: Value) -> Result<Self, FlowError> {
        // Check for compiled format first (has "format": "compiled")
        if let Some(format) = value.get("format").and_then(|f| f.as_str()) {
            if format == "compiled" {
                let mut def: Self = serde_json::from_value(value).map_err(|e| {
                    FlowError::InvalidDefinition(format!("Invalid compiled format: {}", e))
                })?;
                def.build_index();
                return Ok(def);
            }
        }

        // Try to detect format by checking first node's structure
        let is_designer_format = value
            .get("nodes")
            .and_then(|nodes| nodes.as_array())
            .and_then(|arr| arr.first())
            .and_then(|node| node.get("node_type"))
            .is_some();

        if is_designer_format {
            // Designer format - parse and convert
            let designer: DesignerFlowDefinition = serde_json::from_value(value).map_err(|e| {
                FlowError::InvalidDefinition(format!("Failed to parse designer format: {}", e))
            })?;
            let mut def = designer.to_runtime_format();
            def.build_index();
            Ok(def)
        } else {
            // Runtime format - parse directly and generate edges if needed
            let def: FlowDefinition = serde_json::from_value(value).map_err(|e| {
                FlowError::InvalidDefinition(format!("Failed to parse flow definition: {}", e))
            })?;
            // with_generated_edges also builds the index
            Ok(def.with_generated_edges())
        }
    }

    /// Generate edges from `next_node` fields if the edges array is empty.
    ///
    /// This enables simple sequential flows to be defined without explicit edges,
    /// using just the `next_node` field on each node.
    pub fn with_generated_edges(mut self) -> Self {
        if self.edges.is_empty() {
            // Generate edges from next_node fields
            for node in &self.nodes {
                if let Some(next) = &node.next_node {
                    self.edges.push(FlowEdge {
                        from: node.id.clone(),
                        to: next.clone(),
                        label: None,
                        condition: None,
                    });
                }
            }

            // Also generate edges from children's next_node fields
            self.generate_edges_from_children(&self.nodes.clone());
        }
        // Build index for O(1) node lookups
        self.build_index();
        self
    }

    /// Recursively generate edges from children nodes
    fn generate_edges_from_children(&mut self, nodes: &[FlowNode]) {
        for node in nodes {
            for child in &node.children {
                if let Some(next) = &child.next_node {
                    self.edges.push(FlowEdge {
                        from: child.id.clone(),
                        to: next.clone(),
                        label: None,
                        condition: None,
                    });
                }
            }
            // Recurse into children
            if !node.children.is_empty() {
                self.generate_edges_from_children(&node.children);
            }
        }
    }
}

/// Recursively index child nodes
fn index_children(index: &mut HashMap<String, usize>, nodes: &[FlowNode]) {
    for node in nodes {
        for child in &node.children {
            if !index.contains_key(&child.id) {
                index.insert(child.id.clone(), usize::MAX);
            }
        }
        if !node.children.is_empty() {
            index_children(index, &node.children);
        }
    }
}

/// Recursively find a node in children
fn find_in_children<'a>(node_id: &str, nodes: &'a [FlowNode]) -> Option<&'a FlowNode> {
    for node in nodes {
        if let Some(child) = node.children.iter().find(|c| c.id == node_id) {
            return Some(child);
        }
        if let Some(found) = find_in_children(node_id, &node.children) {
            return Some(found);
        }
    }
    None
}
