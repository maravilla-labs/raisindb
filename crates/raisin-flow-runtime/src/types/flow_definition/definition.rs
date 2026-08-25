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

    /// Check every edge in the definition points at a node that exists.
    ///
    /// **Fail fast, before any side effect.** Without this a dangling edge is
    /// discovered only when the executor walks into it — `StepNotFound`, raised
    /// mid-flow, after earlier steps have already charged a card, mailed a
    /// customer or created inbox tasks. A definition is a small, wholly-known
    /// object; there is no reason to learn it is broken halfway through acting
    /// on it.
    ///
    /// Deliberately NOT a cycle check. A backward edge is legitimate — it is how
    /// a draft/critique/revise loop is expressed — and the executor bounds
    /// runaways with its own step budget while an author bounds an intended loop
    /// with `visits.<step_id>`.
    ///
    /// Returns every dangling reference at once rather than the first, so a
    /// mis-wired graph takes one pass to fix.
    pub fn validate(&self) -> Result<(), FlowError> {
        let mut problems: Vec<String> = Vec::new();

        if self.nodes.is_empty() {
            return Err(FlowError::InvalidDefinition(
                "flow has no nodes".to_string(),
            ));
        }

        fn check(def: &FlowDefinition, node: &FlowNode, problems: &mut Vec<String>) {
            // Every way a node can name a successor. `error_edge` and
            // `timeout_edge` are the ones that rot unnoticed: they are only
            // taken on a failure path, so a typo there survives every happy-path
            // test run and surfaces the first time something goes wrong.
            let mut targets: Vec<(&str, String)> = Vec::new();
            if let Some(next) = &node.next_node {
                targets.push(("next_node", next.clone()));
            }
            for key in [
                "yes_branch",
                "no_branch",
                "error_edge",
                "timeout_edge",
                "next_step",
            ] {
                if let Some(target) = node.properties.get(key).and_then(|v| v.as_str()) {
                    targets.push((key, target.to_string()));
                }
            }

            for (key, target) in targets {
                if def.find_node(&target).is_none() {
                    problems.push(format!(
                        "step '{}' has {} -> '{}', which is not a node in this flow",
                        node.id, key, target
                    ));
                }
            }

            // A loop must say WHICH KIND it is. Exactly one of a collection, a
            // condition or a fixed count decides that; naming none leaves the
            // handler with nothing to iterate, and naming several means the
            // engine picks by precedence while the author believes something
            // else. This is checked here rather than at parse time so it covers
            // hand-authored runtime format too, not just the designer lowering.
            if node.step_type == StepType::Loop {
                let named: Vec<&str> = ["collection", "condition", "times"]
                    .into_iter()
                    .filter(|k| node.properties.contains_key(*k))
                    .collect();
                match named.len() {
                    0 => problems.push(format!(
                        "loop '{}' names none of 'over' / 'while' / 'times'",
                        node.id
                    )),
                    1 => {}
                    _ => problems.push(format!(
                        "loop '{}' names {} shapes at once ({}); exactly one decides how it iterates",
                        node.id,
                        named.len(),
                        named.join(", ")
                    )),
                }

                // `unbounded` drops the iteration ceiling, which only means
                // anything for a `while`: a for_each is bounded by its
                // collection and a times by its count, so accepting it there
                // would be accepting a word that does nothing.
                let unbounded = node
                    .properties
                    .get("unbounded")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if unbounded {
                    if !node.properties.contains_key("condition") {
                        problems.push(format!(
                            "loop '{}' is 'unbounded' but is not a 'while' loop; \
                             'over' is bounded by its collection and 'times' by its count",
                            node.id
                        ));
                    }
                    if node.properties.contains_key("max_iterations") {
                        problems.push(format!(
                            "loop '{}' is both 'unbounded' and capped by 'max_iterations'; \
                             pick one",
                            node.id
                        ));
                    }
                }
            }

            for child in &node.children {
                check(def, child, problems);
            }
        }

        for node in &self.nodes {
            check(self, node, &mut problems);
        }

        // A duplicate id silently shadows: `find_node` returns the FIRST match,
        // so every edge aimed at the later one lands on the earlier one instead
        // and the flow runs the wrong step while reporting success.
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        fn check_ids<'a>(
            nodes: &'a [FlowNode],
            seen: &mut std::collections::HashSet<&'a str>,
            problems: &mut Vec<String>,
        ) {
            for node in nodes {
                if !seen.insert(node.id.as_str()) {
                    problems.push(format!("duplicate step id '{}'", node.id));
                }
                check_ids(&node.children, seen, problems);
            }
        }
        check_ids(&self.nodes, &mut seen, &mut problems);

        if problems.is_empty() {
            Ok(())
        } else {
            Err(FlowError::InvalidDefinition(problems.join("; ")))
        }
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

        // Try to detect format by checking the first node's structure.
        // An EMPTY nodes array is treated as designer format too: the
        // designer saves empty flows without start/end nodes, while an
        // empty runtime-format flow would be invalid anyway (no start).
        let is_designer_format = value
            .get("nodes")
            .and_then(|nodes| nodes.as_array())
            .map(|arr| {
                arr.is_empty() || arr.first().and_then(|node| node.get("node_type")).is_some()
            })
            .unwrap_or(false);

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

#[cfg(test)]
mod validate_tests {
    use super::*;

    fn def(v: Value) -> FlowDefinition {
        FlowDefinition::from_workflow_data(v).unwrap()
    }

    /// A dangling edge is caught BEFORE the first step, not walked into.
    #[test]
    fn test_validate_rejects_a_dangling_edge() {
        let d = def(serde_json::json!({
            "nodes": [
                { "id": "start", "step_type": "start", "next_node": "a" },
                { "id": "a", "step_type": "decision",
                  "properties": { "condition": "true", "yes_branch": "b", "no_branch": "typo" } },
                { "id": "b", "step_type": "end" }
            ]
        }));
        let err = d.validate().unwrap_err().to_string();
        assert!(
            err.contains("typo"),
            "should name the missing target: {err}"
        );
        assert!(err.contains("no_branch"), "and which edge: {err}");
    }

    /// `error_edge` / `timeout_edge` are the ones that rot unnoticed: they are
    /// only taken on a failure path, so a typo survives every happy-path run.
    #[test]
    fn test_validate_checks_failure_only_edges() {
        let d = def(serde_json::json!({
            "nodes": [
                { "id": "start", "step_type": "start", "next_node": "a" },
                { "id": "a", "step_type": "function_step",
                  "properties": { "function_ref": "/lib/x", "error_edge": "nowhere" },
                  "next_node": "end" },
                { "id": "end", "step_type": "end" }
            ]
        }));
        let err = d.validate().unwrap_err().to_string();
        assert!(err.contains("error_edge"), "{err}");
        assert!(err.contains("nowhere"), "{err}");
    }

    /// Every problem at once, so a mis-wired graph takes one pass to fix.
    #[test]
    fn test_validate_reports_all_problems_together() {
        let d = def(serde_json::json!({
            "nodes": [
                { "id": "start", "step_type": "start", "next_node": "gone_a" },
                { "id": "x", "step_type": "function_step",
                  "properties": { "function_ref": "/lib/x" }, "next_node": "gone_b" }
            ]
        }));
        let err = d.validate().unwrap_err().to_string();
        assert!(err.contains("gone_a") && err.contains("gone_b"), "{err}");
    }

    /// A BACKWARD edge is legitimate - it is how a revise loop is expressed -
    /// and validation must not mistake it for a defect.
    #[test]
    fn test_validate_accepts_a_cycle() {
        let d = def(serde_json::json!({
            "nodes": [
                { "id": "start", "step_type": "start", "next_node": "draft" },
                { "id": "draft", "step_type": "function_step",
                  "properties": { "function_ref": "/lib/draft" }, "next_node": "critic" },
                { "id": "critic", "step_type": "decision",
                  "properties": { "condition": "visits.draft < 3",
                                  "yes_branch": "draft", "no_branch": "end" } },
                { "id": "end", "step_type": "end" }
            ]
        }));
        assert!(d.validate().is_ok(), "{:?}", d.validate());
    }

    /// A duplicate id silently shadows: `find_node` resolves one of them and
    /// every edge aimed at the other lands on the wrong step.
    #[test]
    fn test_validate_rejects_duplicate_ids() {
        let d = def(serde_json::json!({
            "nodes": [
                { "id": "start", "step_type": "start", "next_node": "twin" },
                { "id": "twin", "step_type": "function_step",
                  "properties": { "function_ref": "/lib/a" }, "next_node": "end" },
                { "id": "twin", "step_type": "function_step",
                  "properties": { "function_ref": "/lib/b" }, "next_node": "end" },
                { "id": "end", "step_type": "end" }
            ]
        }));
        assert!(d
            .validate()
            .unwrap_err()
            .to_string()
            .contains("duplicate step id 'twin'"));
    }

    /// A flow whose own first step is named `start` must not gain a SECOND node
    /// with that id pointing at itself.
    ///
    /// It used to. The lowering injected an implicit Start unconditionally, and
    /// with the author's entry also called `start` its `next_node` resolved to
    /// `start` - itself. That survived only because `build_index` is last-wins,
    /// so lookups landed on the author's node; on the linear fallback path
    /// `find_node` returns the FIRST match and the flow spins forever.
    #[test]
    fn test_author_named_start_is_not_duplicated_by_lowering() {
        let d = def(serde_json::json!({
            "nodes": [
                { "id": "start", "node_type": "raisin:FlowStep",
                  "properties": { "action": "first", "function_ref": "/lib/first" } }
            ]
        }));

        let starts = d.nodes.iter().filter(|n| n.id == "start").count();
        assert_eq!(starts, 1, "exactly one node may claim the id `start`");
        assert!(d.validate().is_ok(), "{:?}", d.validate());

        let entry = d.find_node("start").expect("entry must resolve");
        assert_ne!(
            entry.next_node.as_deref(),
            Some("start"),
            "the entry must not point at itself"
        );
    }
}
