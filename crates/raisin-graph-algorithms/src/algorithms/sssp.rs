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

use crate::projection::GraphProjection;
use ordered_float::OrderedFloat;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

/// Single-Source Shortest Path using Dijkstra's algorithm.
///
/// Computes shortest weighted distances from `source` to all reachable nodes
/// using a binary min-heap on the CSR graph representation. Edge weights are
/// read from the projection's columnar weight storage; unweighted graphs use
/// an implicit weight of 1.0 per edge.
///
/// # Arguments
/// * `projection` - The graph projection with optional edge weights
/// * `source` - The string ID of the source node
///
/// # Returns
/// A `HashMap<String, f64>` mapping each reachable node to its shortest distance.
/// The source node has distance 0.0. Unreachable nodes are excluded from the map.
/// If the source is not in the graph, returns an empty map.
///
/// # Complexity
/// O((V + E) * log(V)) with a binary min-heap.
pub fn sssp(projection: &GraphProjection, source: &str) -> HashMap<String, f64> {
    let node_count = projection.node_count();
    if node_count == 0 {
        return HashMap::new();
    }

    let source_id = match projection.get_id(source) {
        Some(id) => id,
        None => return HashMap::new(),
    };

    let graph = projection.graph();

    // Pre-compute edge offsets: offsets[u] = cumulative edge count for nodes 0..u
    // This lets us map from (node u, k-th neighbor) to the global CSR edge index.
    let mut edge_offsets = vec![0usize; node_count + 1];
    for u in 0..node_count {
        let deg = if u < graph.node_count() {
            graph.neighbors_slice(u as u32).len()
        } else {
            0
        };
        edge_offsets[u + 1] = edge_offsets[u] + deg;
    }

    // Initialize distances
    let mut distances = vec![f64::INFINITY; node_count];
    distances[source_id as usize] = 0.0;

    // Min-heap: (distance, node_id)
    let mut heap: BinaryHeap<Reverse<(OrderedFloat<f64>, u32)>> = BinaryHeap::new();
    heap.push(Reverse((OrderedFloat(0.0), source_id)));

    while let Some(Reverse((OrderedFloat(dist_u), u))) = heap.pop() {
        // Skip stale entries
        if dist_u > distances[u as usize] {
            continue;
        }

        if (u as usize) >= graph.node_count() {
            continue;
        }

        let neighbors = graph.neighbors_slice(u);
        let base_edge_idx = edge_offsets[u as usize];

        for (k, &v) in neighbors.iter().enumerate() {
            let edge_idx = base_edge_idx + k;
            let weight = projection.edge_weight(edge_idx);
            let new_dist = dist_u + weight;

            if new_dist < distances[v as usize] {
                distances[v as usize] = new_dist;
                heap.push(Reverse((OrderedFloat(new_dist), v)));
            }
        }
    }

    // Map back to String IDs, excluding unreachable nodes
    let mut result = HashMap::with_capacity(node_count);
    for (i, &dist) in distances.iter().enumerate() {
        if dist.is_finite() {
            if let Some(node_id) = projection.get_node_id(i as u32) {
                result.insert(node_id.clone(), dist);
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::GraphProjection;

    #[test]
    fn test_sssp_empty_graph() {
        let projection = GraphProjection::from_parts(vec![], vec![]);
        let result = sssp(&projection, "A");
        assert!(result.is_empty());
    }

    #[test]
    fn test_sssp_single_node() {
        let nodes = vec!["A".to_string()];
        let edges: Vec<(String, String, f64)> = vec![];
        let projection = GraphProjection::from_parts_weighted(nodes, edges);

        let result = sssp(&projection, "A");
        assert_eq!(result.len(), 1);
        assert!((result["A"] - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_sssp_weighted_diamond() {
        // A->B (1.0), A->C (4.0), B->C (1.0), B->D (5.0), C->D (1.0)
        // Shortest paths from A: A=0, B=1, C=2, D=3
        let nodes = vec![
            "A".to_string(),
            "B".to_string(),
            "C".to_string(),
            "D".to_string(),
        ];
        let edges = vec![
            ("A".to_string(), "B".to_string(), 1.0),
            ("A".to_string(), "C".to_string(), 4.0),
            ("B".to_string(), "C".to_string(), 1.0),
            ("B".to_string(), "D".to_string(), 5.0),
            ("C".to_string(), "D".to_string(), 1.0),
        ];
        let projection = GraphProjection::from_parts_weighted(nodes, edges);

        let result = sssp(&projection, "A");
        assert!((result["A"] - 0.0).abs() < f64::EPSILON);
        assert!((result["B"] - 1.0).abs() < f64::EPSILON);
        assert!((result["C"] - 2.0).abs() < f64::EPSILON);
        assert!((result["D"] - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_sssp_unit_weights_match_bfs() {
        // Unweighted graph: A -> B -> C -> D
        // All edges have implicit weight 1.0, so SSSP should match BFS distances
        let nodes = vec![
            "A".to_string(),
            "B".to_string(),
            "C".to_string(),
            "D".to_string(),
        ];
        let edges = vec![
            ("A".to_string(), "B".to_string()),
            ("B".to_string(), "C".to_string()),
            ("C".to_string(), "D".to_string()),
        ];
        let projection = GraphProjection::from_parts(nodes, edges);
        assert!(!projection.has_weights());

        let result = sssp(&projection, "A");
        assert!((result["A"] - 0.0).abs() < f64::EPSILON);
        assert!((result["B"] - 1.0).abs() < f64::EPSILON);
        assert!((result["C"] - 2.0).abs() < f64::EPSILON);
        assert!((result["D"] - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_sssp_disconnected() {
        // A -> B, C (isolated)
        let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let edges = vec![("A".to_string(), "B".to_string(), 2.0)];
        let projection = GraphProjection::from_parts_weighted(nodes, edges);

        let result = sssp(&projection, "A");
        assert!((result["A"] - 0.0).abs() < f64::EPSILON);
        assert!((result["B"] - 2.0).abs() < f64::EPSILON);
        // C is unreachable
        assert!(!result.contains_key("C"));
    }

    #[test]
    fn test_sssp_source_not_in_graph() {
        let nodes = vec!["A".to_string(), "B".to_string()];
        let edges = vec![("A".to_string(), "B".to_string(), 1.0)];
        let projection = GraphProjection::from_parts_weighted(nodes, edges);

        let result = sssp(&projection, "Z");
        assert!(result.is_empty());
    }
}
