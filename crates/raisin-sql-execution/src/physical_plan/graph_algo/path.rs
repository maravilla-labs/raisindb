//! Engine-neutral ordered path type.
//!
//! [`GraphPath`] is what every path-producing algorithm in this module
//! returns. It carries the full ordered node and edge sequence — unlike the
//! Cypher `PathInfo` it replaces, which lost the target node type, and unlike
//! the PGQ variable-length matcher which used to throw the ordered sequence
//! away and smuggle the hop count out inside a rewritten relation type.
//!
//! `length()` is a **method**, not a stored field, so it cannot drift away
//! from the edge sequence it describes.
//!
//! This type carries path *semantics* only (length, distinctness, identity).
//! Rendering a path into SQL values — the fixed JSON shapes behind `nodes(p)`
//! and `edges(p)` — is a projection concern and lives in
//! `pgq::filter::path_accessors`, so the wire contract has exactly one
//! definition.

use super::types::{GraphEdge, GraphNodeId};
use std::collections::HashSet;

/// One node on a path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PathNode {
    /// Node identifier.
    pub id: String,
    /// Workspace containing the node.
    pub workspace: String,
    /// Node type (may be empty when the adjacency source does not track it).
    pub node_type: String,
}

impl PathNode {
    /// Build a path node.
    pub fn new(
        id: impl Into<String>,
        workspace: impl Into<String>,
        node_type: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            workspace: workspace.into(),
            node_type: node_type.into(),
        }
    }

    /// Build a path node from an adjacency key, with no known node type.
    pub fn from_key(key: &GraphNodeId) -> Self {
        Self::new(key.1.clone(), key.0.clone(), String::new())
    }

    /// The `(workspace, id)` adjacency key for this node.
    pub fn key(&self) -> GraphNodeId {
        (self.workspace.clone(), self.id.clone())
    }
}

/// One edge on a path.
#[derive(Debug, Clone, PartialEq)]
pub struct PathEdge {
    /// Semantic relation type.
    pub relation_type: String,
    /// Identifier of the source node.
    pub source_id: String,
    /// Identifier of the target node.
    pub target_id: String,
    /// Workspace of the source node.
    pub source_workspace: String,
    /// Workspace of the target node.
    pub target_workspace: String,
    /// Edge weight verbatim from `RelationRef::weight`; `None` when unset.
    pub weight: Option<f32>,
}

impl PathEdge {
    /// The edge identity used for TRAIL (edge-distinct) checking.
    ///
    /// Weight is deliberately excluded: two edges differing only by weight are
    /// the same edge.
    pub fn identity(&self) -> (&str, &str, &str, &str, &str) {
        (
            &self.source_workspace,
            &self.source_id,
            &self.target_workspace,
            &self.target_id,
            &self.relation_type,
        )
    }
}

/// An ordered path through the graph.
///
/// Invariant: `nodes.len() == edges.len() + 1` for every path built through
/// [`GraphPath::start`] and [`GraphPath::extended`].
#[derive(Debug, Clone, PartialEq)]
pub struct GraphPath {
    /// Nodes in path order.
    pub nodes: Vec<PathNode>,
    /// Edges in path order; `edges[i]` connects `nodes[i]` to `nodes[i + 1]`.
    pub edges: Vec<PathEdge>,
}

impl GraphPath {
    /// Start a zero-hop path at `node`.
    pub fn start(node: PathNode) -> Self {
        Self {
            nodes: vec![node],
            edges: Vec::new(),
        }
    }

    /// Start a zero-hop path at an adjacency key (node type unknown).
    pub fn from_key(key: &GraphNodeId) -> Self {
        Self::start(PathNode::from_key(key))
    }

    /// Replace the start node's type.
    ///
    /// Algorithms take `(workspace, id)` keys and therefore cannot know the
    /// start node's type; a caller that has it bound supplies it here.
    pub fn with_start_node_type(mut self, node_type: impl Into<String>) -> Self {
        if let Some(first) = self.nodes.first_mut() {
            first.node_type = node_type.into();
        }
        self
    }

    /// Number of hops (== `edges.len()`).
    pub fn length(&self) -> usize {
        self.edges.len()
    }

    /// The first node on the path.
    pub fn first(&self) -> Option<&PathNode> {
        self.nodes.first()
    }

    /// The last node on the path.
    pub fn last(&self) -> Option<&PathNode> {
        self.nodes.last()
    }

    /// Clone this path extended by one hop along `edge`.
    pub fn extended(&self, edge: &GraphEdge) -> Self {
        let mut next = self.clone();
        next.push(edge);
        next
    }

    /// Extend this path in place by one hop along `edge`.
    pub fn push(&mut self, edge: &GraphEdge) {
        let (source_workspace, source_id) = match self.nodes.last() {
            Some(node) => (node.workspace.clone(), node.id.clone()),
            None => (String::new(), String::new()),
        };
        self.edges.push(PathEdge {
            relation_type: edge.relation_type.clone(),
            source_id,
            target_id: edge.target_id.clone(),
            source_workspace,
            target_workspace: edge.target_workspace.clone(),
            weight: edge.weight,
        });
        self.nodes.push(PathNode::new(
            edge.target_id.clone(),
            edge.target_workspace.clone(),
            edge.target_node_type.clone(),
        ));
    }

    /// Whether a node already appears on this path (ACYCLIC check).
    ///
    /// **Argument order is `(id, workspace)`** — the historical order inherited
    /// from the Cypher `PathInfo` this type replaced, kept so no call site
    /// changed meaning during the move. Note this is the OPPOSITE of
    /// [`GraphNodeId`], which is `(workspace, id)`; both parameters are `&str`
    /// so a swap compiles silently. Prefer [`GraphPath::contains_key`] when you
    /// already hold a [`GraphNodeId`].
    pub fn contains_node(&self, id: &str, workspace: &str) -> bool {
        self.nodes
            .iter()
            .any(|n| n.id == id && n.workspace == workspace)
    }

    /// Whether `key` already appears on this path.
    ///
    /// The unambiguous form of [`GraphPath::contains_node`].
    pub fn contains_key(&self, key: &GraphNodeId) -> bool {
        self.nodes
            .iter()
            .any(|n| n.workspace == key.0 && n.id == key.1)
    }

    /// Whether extending along `edge` from the path's current end would repeat
    /// an edge already on this path (TRAIL check, adjacency form).
    pub fn would_repeat_edge(&self, edge: &GraphEdge) -> bool {
        let Some(source) = self.nodes.last() else {
            return false;
        };
        self.edges.iter().any(|e| {
            e.source_workspace == source.workspace
                && e.source_id == source.id
                && e.target_workspace == edge.target_workspace
                && e.target_id == edge.target_id
                && e.relation_type == edge.relation_type
        })
    }

    /// TRAIL: every edge on the path is distinct.
    pub fn is_trail(&self) -> bool {
        let mut seen = HashSet::new();
        self.edges.iter().all(|e| seen.insert(e.identity()))
    }

    /// ACYCLIC: every node on the path is distinct.
    pub fn is_acyclic(&self) -> bool {
        let mut seen = HashSet::new();
        self.nodes
            .iter()
            .all(|n| seen.insert((&n.workspace, &n.id)))
    }

    /// Total weight of the path, or `None` if any edge is unweighted.
    pub fn total_weight(&self) -> Option<f64> {
        self.edges
            .iter()
            .map(|e| e.weight.map(|w| w as f64))
            .sum::<Option<f64>>()
    }

    /// Opaque, stable encoding of the whole path.
    ///
    /// Shape: `"<ws>:<id>|<rel>|<ws>:<id>|<rel>|..."`, node/edge alternating.
    /// Usable as an equality or dedup key. **Not** a parseable contract.
    pub fn element_id(&self) -> String {
        let mut out = String::new();
        for (i, node) in self.nodes.iter().enumerate() {
            if i > 0 {
                out.push('|');
                out.push_str(&self.edges[i - 1].relation_type);
                out.push('|');
            }
            out.push_str(&node.workspace);
            out.push(':');
            out.push_str(&node.id);
        }
        out
    }
}
