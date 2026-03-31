//! Shared type aliases for graph algorithm implementations
//!
//! These type aliases reduce complexity in function signatures across
//! all graph algorithm modules (shortest path, centrality, community detection, etc.).

use std::collections::HashMap;

/// A graph node identifier: (workspace, node_id)
pub type GraphNodeId = (String, String);

/// A directed graph edge: (target_workspace, target_id, relation_type)
pub type GraphEdge = (String, String, String);

/// Adjacency list representation of a directed graph.
///
/// Maps each node to its outgoing edges.
pub type GraphAdjacency = HashMap<GraphNodeId, Vec<GraphEdge>>;

/// BFS visited map with optional parent tracking.
///
/// Each entry maps a visited node to its predecessor and the relation type
/// used to reach it. The starting node maps to `None`.
pub type BfsVisited = HashMap<GraphNodeId, Option<(GraphNodeId, String)>>;

/// A path represented as a sequence of indexed edges: (source_idx, target_idx, relation_type).
///
/// Used internally by algorithms that map nodes to integer indices for efficiency.
pub type IndexedPath = Vec<(usize, usize, String)>;

/// An indexed path together with its total cost.
pub type WeightedIndexedPath = (f64, IndexedPath);
