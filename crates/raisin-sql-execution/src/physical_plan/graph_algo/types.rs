//! Shared adjacency types for the engine-neutral graph algorithms.
//!
//! These are used by BOTH the Cypher and the SQL/PGQ execution paths. The
//! shape is the PGQ one on purpose: PGQ needs `target_node_type` for label
//! filtering and `weight` for `ANY CHEAPEST`, and a type that carried neither
//! would force one of the two engines to keep a private copy of every
//! algorithm — which is exactly what this module exists to prevent.
//!
//! Callers whose source of truth has no node type or weight (the Cypher
//! adjacency builders) use [`GraphEdge::untyped`].

use std::collections::HashMap;

/// A graph node identifier: `(workspace, node_id)`.
pub type GraphNodeId = (String, String);

/// A directed graph edge in an adjacency list.
///
/// Mirrors the fields a [`raisin_models::nodes::RelationRef`] actually has.
/// A relation carries **no property map** — `weight` is the only numeric
/// attribute an edge has, which is why `COST` may only reference it.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphEdge {
    /// Workspace containing the target node.
    pub target_workspace: String,
    /// Identifier of the target node.
    pub target_id: String,
    /// Node type of the target (empty when the builder does not track it).
    pub target_node_type: String,
    /// Semantic relation type (e.g. `knows`, `Transfers`).
    pub relation_type: String,
    /// Optional edge weight, verbatim from `RelationRef::weight`.
    pub weight: Option<f32>,
}

impl GraphEdge {
    /// Build a fully-specified edge.
    pub fn new(
        target_workspace: impl Into<String>,
        target_id: impl Into<String>,
        target_node_type: impl Into<String>,
        relation_type: impl Into<String>,
        weight: Option<f32>,
    ) -> Self {
        Self {
            target_workspace: target_workspace.into(),
            target_id: target_id.into(),
            target_node_type: target_node_type.into(),
            relation_type: relation_type.into(),
            weight,
        }
    }

    /// Build an edge with no target node type and no weight.
    ///
    /// For adjacency builders that do not track them (the Cypher surface).
    /// An untyped edge is unusable under `ANY CHEAPEST`, which is correct:
    /// a missing weight must be an error, never a silent `1.0`.
    pub fn untyped(
        target_workspace: impl Into<String>,
        target_id: impl Into<String>,
        relation_type: impl Into<String>,
    ) -> Self {
        Self::new(
            target_workspace,
            target_id,
            String::new(),
            relation_type,
            None,
        )
    }

    /// The `(workspace, id)` key of this edge's target.
    pub fn target(&self) -> GraphNodeId {
        (self.target_workspace.clone(), self.target_id.clone())
    }
}

/// Adjacency list representation of a directed graph.
///
/// Maps each node to its outgoing edges.
pub type GraphAdjacency = HashMap<GraphNodeId, Vec<GraphEdge>>;

/// BFS visited map with optional parent tracking.
///
/// Each entry maps a visited node to its predecessor and the relation type
/// used to reach it. The starting node maps to `None`.
pub type BfsVisited = HashMap<GraphNodeId, Option<(GraphNodeId, String)>>;

/// A path represented as a sequence of indexed edges: `(source_idx, target_idx, relation_type)`.
///
/// Used internally by algorithms that map nodes to integer indices for efficiency.
pub type IndexedPath = Vec<(usize, usize, String)>;

/// An indexed path together with its total cost.
pub type WeightedIndexedPath = (f64, IndexedPath);
