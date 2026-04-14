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

use crate::error::{GraphError, Result};
use petgraph::csr::Csr;
use raisin_hlc::HLC;
use raisin_storage::{RelationRepository, Storage};
use std::collections::HashMap;
use std::sync::Arc;

#[cfg(test)]
mod tests;

/// A read-only, in-memory projection of a subgraph.
/// Optimized for algorithm execution (integer IDs, contiguous memory).
pub struct GraphProjection {
    /// The underlying graph structure (Compressed Sparse Row) - forward edges
    graph: Csr<(), ()>,

    /// Transpose graph (backward edges) for pull-based algorithms (e.g., PageRank).
    /// Built lazily on first call to `ensure_backward_graph()`.
    backward_graph: Option<Csr<(), ()>>,

    /// Columnar edge weights aligned to forward CSR edge ordering.
    /// `edge_weights[i]` is the weight of the i-th edge in the CSR structure.
    /// `None` means unweighted graph (all edges have implicit weight 1.0).
    edge_weights: Option<Vec<f64>>,

    /// Map from String ID (RaisinDB) to Integer ID (Projection)
    id_map: HashMap<String, u32>,

    /// Map from Integer ID (Projection) back to String ID (RaisinDB)
    reverse_map: Vec<String>,
}

impl Default for GraphProjection {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphProjection {
    /// Create a new empty projection
    pub fn new() -> Self {
        Self {
            graph: Csr::new(),
            backward_graph: None,
            edge_weights: None,
            id_map: HashMap::new(),
            reverse_map: Vec::new(),
        }
    }

    /// Build a projection from a list of nodes and edges (unweighted).
    pub fn from_parts(nodes: Vec<String>, edges: Vec<(String, String)>) -> Self {
        let mut id_map = HashMap::with_capacity(nodes.len());
        let mut reverse_map = Vec::with_capacity(nodes.len());

        // 1. Build ID mapping
        for (i, node_id) in nodes.into_iter().enumerate() {
            id_map.insert(node_id.clone(), i as u32);
            reverse_map.push(node_id);
        }

        // 2. Build Edge list with integer IDs
        let mut edge_list = Vec::with_capacity(edges.len());
        for (src, dst) in edges {
            if let (Some(&u), Some(&v)) = (id_map.get(&src), id_map.get(&dst)) {
                edge_list.push((u, v));
            }
        }

        // 3. Construct CSR graph
        edge_list.sort_unstable();
        let graph: Csr<(), ()> = Csr::from_sorted_edges(&edge_list).unwrap_or_default();

        Self {
            graph,
            backward_graph: None,
            edge_weights: None,
            id_map,
            reverse_map,
        }
    }

    /// Build a projection from a list of nodes and weighted edges.
    ///
    /// Edge weights are stored in a columnar layout aligned to the CSR edge ordering.
    /// This enables O(1) weight lookup by edge index for algorithms like SSSP.
    pub fn from_parts_weighted(nodes: Vec<String>, edges: Vec<(String, String, f64)>) -> Self {
        let mut id_map = HashMap::with_capacity(nodes.len());
        let mut reverse_map = Vec::with_capacity(nodes.len());

        for (i, node_id) in nodes.into_iter().enumerate() {
            id_map.insert(node_id.clone(), i as u32);
            reverse_map.push(node_id);
        }

        // Build edge list with integer IDs and weights
        let mut edge_list_with_weights: Vec<(u32, u32, f64)> = Vec::with_capacity(edges.len());
        for (src, dst, weight) in edges {
            if let (Some(&u), Some(&v)) = (id_map.get(&src), id_map.get(&dst)) {
                edge_list_with_weights.push((u, v, weight));
            }
        }

        // Sort by (source, target) to match CSR ordering
        edge_list_with_weights.sort_unstable_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));

        // Split into topology and weights
        let mut edge_list = Vec::with_capacity(edge_list_with_weights.len());
        let mut weights = Vec::with_capacity(edge_list_with_weights.len());
        for (u, v, w) in edge_list_with_weights {
            edge_list.push((u, v));
            weights.push(w);
        }

        let graph: Csr<(), ()> = Csr::from_sorted_edges(&edge_list).unwrap_or_default();

        Self {
            graph,
            backward_graph: None,
            edge_weights: Some(weights),
            id_map,
            reverse_map,
        }
    }

    /// Build a projection from storage by scanning relationships
    pub async fn from_storage<S: Storage>(
        storage: &Arc<S>,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        relation_type: Option<&str>,
        max_revision: Option<&HLC>,
    ) -> Result<Self> {
        let relations = storage
            .relations()
            .scan_relations_global(
                raisin_storage::scope::BranchScope::new(tenant_id, repo_id, branch),
                relation_type,
                max_revision,
            )
            .await
            .map_err(|e| GraphError::Storage(e.to_string()))?;

        let mut unique_nodes = std::collections::HashSet::new();
        let mut edges = Vec::with_capacity(relations.len());

        for (_, source_id, _, target_id, _) in relations {
            unique_nodes.insert(source_id.clone());
            unique_nodes.insert(target_id.clone());
            edges.push((source_id, target_id));
        }

        let mut nodes: Vec<String> = unique_nodes.into_iter().collect();
        nodes.sort();
        Ok(Self::from_parts(nodes, edges))
    }

    /// Build a projection from storage, extracting edge weights from relations.
    ///
    /// Relations with `weight: Some(w)` get that weight; others get 1.0.
    pub async fn from_storage_weighted<S: Storage>(
        storage: &Arc<S>,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        relation_type: Option<&str>,
        max_revision: Option<&HLC>,
    ) -> Result<Self> {
        let relations = storage
            .relations()
            .scan_relations_global(
                raisin_storage::scope::BranchScope::new(tenant_id, repo_id, branch),
                relation_type,
                max_revision,
            )
            .await
            .map_err(|e| GraphError::Storage(e.to_string()))?;

        let mut unique_nodes = std::collections::HashSet::new();
        let mut edges = Vec::with_capacity(relations.len());
        let mut has_weights = false;

        for (_, source_id, _, target_id, rel) in &relations {
            unique_nodes.insert(source_id.clone());
            unique_nodes.insert(target_id.clone());
            let weight = rel.weight.map(|w| w as f64).unwrap_or(1.0);
            if rel.weight.is_some() {
                has_weights = true;
            }
            edges.push((source_id.clone(), target_id.clone(), weight));
        }

        let mut nodes: Vec<String> = unique_nodes.into_iter().collect();
        nodes.sort();

        if has_weights {
            Ok(Self::from_parts_weighted(nodes, edges))
        } else {
            // No weights found - use unweighted path for zero overhead
            let unweighted_edges: Vec<(String, String)> =
                edges.into_iter().map(|(s, t, _)| (s, t)).collect();
            Ok(Self::from_parts(nodes, unweighted_edges))
        }
    }

    /// Lazily build the backward (transpose) graph.
    ///
    /// The backward graph has all edges reversed: if the forward graph has edge u->v,
    /// the backward graph has edge v->u. This enables pull-based algorithms where each
    /// node reads from its in-neighbors without write contention.
    ///
    /// Cost: O(E log E) one-time. Amortized across all pull-based algorithm runs.
    pub fn ensure_backward_graph(&mut self) {
        if self.backward_graph.is_some() {
            return;
        }

        let node_count = self.reverse_map.len();
        let mut reversed_edges = Vec::with_capacity(self.graph.edge_count());

        for u in 0..node_count {
            if u < self.graph.node_count() {
                for &v in self.graph.neighbors_slice(u as u32) {
                    reversed_edges.push((v, u as u32)); // Flip: v -> u
                }
            }
        }

        reversed_edges.sort_unstable();
        let backward = Csr::from_sorted_edges(&reversed_edges).unwrap_or_default();
        self.backward_graph = Some(backward);
    }

    /// Get the integer ID for a string ID
    pub fn get_id(&self, node_id: &str) -> Option<u32> {
        self.id_map.get(node_id).copied()
    }

    /// Get the string ID for an integer ID
    pub fn get_node_id(&self, id: u32) -> Option<&String> {
        self.reverse_map.get(id as usize)
    }

    /// Get the underlying forward graph reference
    pub fn graph(&self) -> &Csr<(), ()> {
        &self.graph
    }

    /// Get the backward (transpose) graph, if built.
    ///
    /// Call `ensure_backward_graph()` first. In the backward graph,
    /// `neighbors_slice(v)` returns all nodes u that have edge u->v in the forward graph.
    pub fn backward_graph(&self) -> Option<&Csr<(), ()>> {
        self.backward_graph.as_ref()
    }

    /// Get the weight of an edge by its CSR edge index.
    ///
    /// Returns 1.0 if the graph is unweighted or the index is out of bounds.
    /// Edge indices correspond to the sorted edge ordering used during CSR construction.
    pub fn edge_weight(&self, edge_idx: usize) -> f64 {
        self.edge_weights
            .as_ref()
            .and_then(|w| w.get(edge_idx))
            .copied()
            .unwrap_or(1.0)
    }

    /// Check if this projection has edge weights.
    pub fn has_weights(&self) -> bool {
        self.edge_weights.is_some()
    }

    /// Get the edge weights slice (if present).
    pub fn edge_weights(&self) -> Option<&[f64]> {
        self.edge_weights.as_deref()
    }

    /// Number of nodes in the projection
    pub fn node_count(&self) -> usize {
        self.reverse_map.len()
    }

    /// Number of edges in the projection
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }
}
