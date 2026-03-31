// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Flow Compiler
//!
//! Compiles designer format flows into an optimized runtime format.
//! The compiled format includes:
//! - Flattened node array (no nested children)
//! - Explicit edge list
//! - Precomputed execution order (topological sort)
//! - Hash for cache invalidation

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};

use crate::types::designer_format::DesignerFlowDefinition;
use crate::types::flow_definition::{FlowDefinition, FlowEdge, FlowMetadata, FlowNode};

/// Compiled flow definition - optimized for runtime execution.
///
/// This format:
/// - Has all nodes in a flat array (no nested children)
/// - Has explicit edges array
/// - Has precomputed execution order
/// - Includes a hash for cache invalidation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledFlow {
    /// Schema version
    pub version: u32,

    /// Format identifier - always "compiled"
    pub format: String,

    /// All nodes in flat array
    pub nodes: Vec<FlowNode>,

    /// Edges connecting nodes
    pub edges: Vec<FlowEdge>,

    /// Precomputed execution order (topological sort)
    pub execution_order: Vec<String>,

    /// Compilation metadata
    pub metadata: CompiledMetadata,
}

/// Metadata about the compilation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledMetadata {
    /// When the flow was compiled
    pub compiled_at: DateTime<Utc>,

    /// SHA256 hash of the source flow for cache invalidation
    pub hash: String,

    /// Version of the source flow
    pub source_version: u32,

    /// Original flow name (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flow_name: Option<String>,
}

/// Flow compiler - converts designer format to optimized runtime format.
pub struct FlowCompiler;

impl FlowCompiler {
    /// Compile a designer format flow into optimized runtime format.
    ///
    /// # Arguments
    /// * `designer` - The designer format flow definition
    ///
    /// # Returns
    /// A compiled flow ready for efficient runtime execution
    pub fn compile(designer: &DesignerFlowDefinition) -> CompiledFlow {
        // First convert to runtime format to get nodes and edges
        let runtime = designer.clone().to_runtime_format();

        // Flatten all nodes (including children)
        let mut flat_nodes = Vec::new();
        let mut flat_edges = runtime.edges.clone();
        Self::flatten_nodes(&runtime.nodes, &mut flat_nodes, &mut flat_edges);

        // Compute execution order via topological sort
        let execution_order = Self::topological_sort(&flat_nodes, &flat_edges);

        // Compute hash of the source for cache invalidation
        let hash = Self::compute_hash(designer);

        CompiledFlow {
            version: 1,
            format: "compiled".to_string(),
            nodes: flat_nodes,
            edges: flat_edges,
            execution_order,
            metadata: CompiledMetadata {
                compiled_at: Utc::now(),
                hash,
                source_version: designer.version,
                flow_name: None,
            },
        }
    }

    /// Compile from runtime format (when designer format not available)
    pub fn compile_from_runtime(runtime: &FlowDefinition) -> CompiledFlow {
        // Flatten all nodes
        let mut flat_nodes = Vec::new();
        let mut flat_edges = runtime.edges.clone();
        Self::flatten_nodes(&runtime.nodes, &mut flat_nodes, &mut flat_edges);

        // Compute execution order
        let execution_order = Self::topological_sort(&flat_nodes, &flat_edges);

        // Compute hash
        let hash = Self::compute_hash_from_runtime(runtime);

        CompiledFlow {
            version: 1,
            format: "compiled".to_string(),
            nodes: flat_nodes,
            edges: flat_edges,
            execution_order,
            metadata: CompiledMetadata {
                compiled_at: Utc::now(),
                hash,
                source_version: 1,
                flow_name: runtime.metadata.name.clone(),
            },
        }
    }

    /// Flatten nested nodes into a single array.
    ///
    /// Containers with children are kept, but we also add edges
    /// to represent the parent-child relationships.
    fn flatten_nodes(nodes: &[FlowNode], output: &mut Vec<FlowNode>, edges: &mut Vec<FlowEdge>) {
        for node in nodes {
            // Add the node itself (without children for flat representation)
            let flat_node = node.clone();

            // Process children recursively
            if !node.children.is_empty() {
                // Add edges from parent to first child and between children
                let child_ids: Vec<String> = node.children.iter().map(|c| c.id.clone()).collect();

                // Edge from parent to first child
                if let Some(first_child) = child_ids.first() {
                    edges.push(FlowEdge {
                        from: node.id.clone(),
                        to: first_child.clone(),
                        label: Some("child".to_string()),
                        condition: None,
                    });
                }

                // Flatten children
                Self::flatten_nodes(&node.children, output, edges);

                // Clear children from the flat node (they're now separate)
                // Actually, keep children for container execution logic
            }

            output.push(flat_node);
        }
    }

    /// Perform topological sort using Kahn's algorithm.
    ///
    /// Returns nodes in execution order (dependencies before dependents).
    fn topological_sort(nodes: &[FlowNode], edges: &[FlowEdge]) -> Vec<String> {
        // Build adjacency list and in-degree count
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
        let mut all_nodes: HashSet<String> = HashSet::new();

        // Initialize all nodes with 0 in-degree
        for node in nodes {
            all_nodes.insert(node.id.clone());
            in_degree.entry(node.id.clone()).or_insert(0);
            adjacency.entry(node.id.clone()).or_default();
        }

        // Build graph from edges
        for edge in edges {
            // Skip child edges for ordering purposes
            if edge.label.as_deref() == Some("child") {
                continue;
            }

            *in_degree.entry(edge.to.clone()).or_insert(0) += 1;
            adjacency
                .entry(edge.from.clone())
                .or_default()
                .push(edge.to.clone());
        }

        // Kahn's algorithm
        let mut queue: VecDeque<String> = VecDeque::new();
        let mut result: Vec<String> = Vec::new();

        // Start with nodes that have no incoming edges
        for (node_id, &degree) in &in_degree {
            if degree == 0 {
                queue.push_back(node_id.clone());
            }
        }

        while let Some(node_id) = queue.pop_front() {
            result.push(node_id.clone());

            if let Some(neighbors) = adjacency.get(&node_id) {
                for neighbor in neighbors {
                    if let Some(degree) = in_degree.get_mut(neighbor) {
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push_back(neighbor.clone());
                        }
                    }
                }
            }
        }

        // If we didn't process all nodes, there's a cycle
        // In that case, add remaining nodes in arbitrary order
        for node_id in &all_nodes {
            if !result.contains(node_id) {
                result.push(node_id.clone());
            }
        }

        result
    }

    /// Compute SHA256 hash of the designer flow for cache invalidation.
    fn compute_hash(designer: &DesignerFlowDefinition) -> String {
        let json = serde_json::to_string(designer).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(json.as_bytes());
        let result = hasher.finalize();
        hex::encode(result)
    }

    /// Compute SHA256 hash of runtime flow.
    fn compute_hash_from_runtime(runtime: &FlowDefinition) -> String {
        let json = serde_json::to_string(runtime).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(json.as_bytes());
        let result = hasher.finalize();
        hex::encode(result)
    }
}

impl CompiledFlow {
    /// Convert to FlowDefinition for execution.
    pub fn to_flow_definition(&self) -> FlowDefinition {
        let mut def = FlowDefinition {
            nodes: self.nodes.clone(),
            edges: self.edges.clone(),
            metadata: FlowMetadata {
                name: self.metadata.flow_name.clone(),
                ..Default::default()
            },
            node_index: None,
        };
        def.build_index();
        def
    }

    /// Check if this compiled flow is still valid for the given source hash.
    pub fn is_valid_for_hash(&self, source_hash: &str) -> bool {
        self.metadata.hash == source_hash
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::designer_format::{
        DesignerAiConfig, DesignerContainerType, DesignerNode, DesignerStepProperties,
    };

    #[test]
    fn test_compile_simple_flow() {
        let designer = DesignerFlowDefinition {
            version: 1,
            error_strategy: Default::default(),
            timeout_ms: None,
            nodes: vec![
                DesignerNode::Step {
                    id: "step_1".to_string(),
                    properties: DesignerStepProperties::default(),
                    on_error: None,
                    error_edge: None,
                },
                DesignerNode::Step {
                    id: "step_2".to_string(),
                    properties: DesignerStepProperties::default(),
                    on_error: None,
                    error_edge: None,
                },
            ],
        };

        let compiled = FlowCompiler::compile(&designer);

        assert_eq!(compiled.format, "compiled");
        // 2 steps + implicit Start + implicit End = 4 nodes
        assert_eq!(compiled.nodes.len(), 4);
        assert!(!compiled.metadata.hash.is_empty());
        // execution_order includes all nodes
        assert_eq!(compiled.execution_order.len(), 4);
    }

    #[test]
    fn test_topological_sort() {
        let nodes = vec![
            FlowNode {
                id: "a".to_string(),
                step_type: crate::types::StepType::Start,
                properties: Default::default(),
                children: vec![],
                next_node: None,
            },
            FlowNode {
                id: "b".to_string(),
                step_type: crate::types::StepType::FunctionStep,
                properties: Default::default(),
                children: vec![],
                next_node: None,
            },
            FlowNode {
                id: "c".to_string(),
                step_type: crate::types::StepType::End,
                properties: Default::default(),
                children: vec![],
                next_node: None,
            },
        ];

        let edges = vec![
            FlowEdge {
                from: "a".to_string(),
                to: "b".to_string(),
                label: None,
                condition: None,
            },
            FlowEdge {
                from: "b".to_string(),
                to: "c".to_string(),
                label: None,
                condition: None,
            },
        ];

        let order = FlowCompiler::topological_sort(&nodes, &edges);

        // a should come before b, b should come before c
        let a_pos = order.iter().position(|x| x == "a").unwrap();
        let b_pos = order.iter().position(|x| x == "b").unwrap();
        let c_pos = order.iter().position(|x| x == "c").unwrap();

        assert!(a_pos < b_pos);
        assert!(b_pos < c_pos);
    }
}
