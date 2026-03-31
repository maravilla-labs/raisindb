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

/// A read-only, in-memory projection of a subgraph.
/// Optimized for algorithm execution (integer IDs, contiguous memory).
pub struct GraphProjection {
    /// The underlying graph structure (Compressed Sparse Row)
    /// NodeIndex is u32 by default in petgraph
    graph: Csr<(), ()>, // We don't store weights/data on edges for now, just topology

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
            id_map: HashMap::new(),
            reverse_map: Vec::new(),
        }
    }

    /// Build a projection from a list of nodes and edges.
    ///
    /// In a real implementation, this would take an Iterator from the Storage layer.
    /// For now, we accept vectors of strings.
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
            // Ignore edges where endpoints are not in the node list (subgraph filtering)
        }

        // 3. Construct CSR graph
        // Sort edges for Csr::from_sorted_edges
        edge_list.sort_unstable();
        let graph: Csr<(), ()> = Csr::from_sorted_edges(&edge_list).unwrap_or_default();

        Self {
            graph,
            id_map,
            reverse_map,
        }
    }

    /// Build a projection from storage by scanning relationships
    ///
    /// This method fetches relationships from the storage layer and builds the in-memory graph.
    /// It supports filtering by relation type.
    pub async fn from_storage<S: Storage>(
        storage: &Arc<S>,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        relation_type: Option<&str>,
        max_revision: Option<&HLC>,
    ) -> Result<Self> {
        // 1. Scan global relationships
        let relations = storage
            .relations()
            .scan_relations_global(
                raisin_storage::scope::BranchScope::new(tenant_id, repo_id, branch),
                relation_type,
                max_revision,
            )
            .await
            .map_err(|e| GraphError::Storage(e.to_string()))?;

        // 2. Collect unique nodes
        let mut unique_nodes = std::collections::HashSet::new();
        let mut edges = Vec::with_capacity(relations.len());

        for (_, source_id, _, target_id, _) in relations {
            unique_nodes.insert(source_id.clone());
            unique_nodes.insert(target_id.clone());
            edges.push((source_id, target_id));
        }

        let nodes: Vec<String> = unique_nodes.into_iter().collect();

        // 3. Build projection
        Ok(Self::from_parts(nodes, edges))
    }

    /// Get the integer ID for a string ID
    pub fn get_id(&self, node_id: &str) -> Option<u32> {
        self.id_map.get(node_id).copied()
    }

    /// Get the string ID for an integer ID
    pub fn get_node_id(&self, id: u32) -> Option<&String> {
        self.reverse_map.get(id as usize)
    }

    /// Get the underlying graph reference
    pub fn graph(&self) -> &Csr<(), ()> {
        &self.graph
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
